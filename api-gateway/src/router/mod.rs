use crate::auth::controller;
use crate::auth::middleware::auth_middleware;
use crate::proxy::ServiceProxy;
use cache::{PresenceCache, SessionCache};
use axum::{
    body::Body,
    extract::{Extension, Json, State},
    http::{Request, StatusCode},
    middleware,
    response::IntoResponse,
    routing::{any, get, post},
    Router,
};
use contract::proto::user::user_service_client::UserServiceClient;
use runtime::{
    config::AppConfig,
    configs::routes_config::RouteRule,
    grpc_client::base::get_rpc_client,
    grpc_client::base::service_names as sn,
    service_discovery::LbWithServiceDiscovery,
};
use oss::{oss, BucketType};
use serde::{Deserialize};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tracing::{error, info};
use uuid::Uuid;
use runtime::auth::Claims;
use crate::proxy::services::common::success_response;
use crate::error::GatewayError;

// 获取上传签名请求参数
#[derive(Debug, Deserialize)]
pub struct GetUploadSignatureRequest {
    pub content_type: String,
    pub bucket_type: Option<String>, // "file" 或 "avatar"，默认"file"
}

/// 构建应用路由
///
/// 这个函数负责构建整个应用的路由系统，包括：
/// - 认证路由（登录、注册、刷新令牌等）
/// - 受保护的API路由（需要JWT验证）
/// - 健康检查和指标端点
/// - 文件上传路由
///
/// # 参数
/// * `service_proxy` - 用于代理服务请求的代理器
/// * `gateway_config` - 网关配置
/// * `cache_instance` - 缓存实例，用于Redis操作
///
/// # 返回值
/// * `Result<Router, anyhow::Error>` - 成功时返回构建好的路由器，失败时返回错误
pub async fn build_routes(
    service_proxy: ServiceProxy,
    config: Arc<AppConfig>,
    cache_instance: Arc<cache::RedisCache>,
) -> anyhow::Result<Router> {
    let gateway_config = &config.gateway;
    // 创建用户服务客户端
    let service_client = get_rpc_client::<UserServiceClient<LbWithServiceDiscovery>>(
        &config,
        sn::RELATION.to_string(),
    )
    .await?;

    let user_service = controller::SharedUserService::new(service_client);

    // 构建基础路由
    // State 统一为 Arc<AppConfig>，由函数末尾 with_state 注入，
    // 这样 handler 与中间件都不必回读全局单例。
    let mut router = Router::<Arc<AppConfig>>::new()
        // 健康检查和指标
        .route("/health", get(health_check))
        .route(&gateway_config.metrics_endpoint, get(crate::metrics::get_metrics_handler))
        // 认证路由（无需认证）
        .route("/api/user/login", post(controller::login))
        .route("/api/user/loginByPhone", post(controller::login_by_phone))
        .route("/api/user/refresh", post(controller::refresh_token));

    // 需要认证的路由
    let authenticated_routes = Router::<Arc<AppConfig>>::new()
        // 用户管理路由
        .route("/api/user/logout", post(controller::logout))
        // 好友在线状态查询路由
        .route("/api/friends/online-status", post(controller::batch_get_friends_online_status))
        // 文件上传签名路由（需要认证以获取租户ID）
        .route("/api/files/upload-signature", post(get_upload_signature))
        .layer(Extension(cache_instance.clone() as Arc<dyn SessionCache>))
        .layer(Extension(cache_instance.clone() as Arc<dyn PresenceCache>))
        .layer(middleware::from_fn_with_state(
            config.clone(),
            auth_middleware,
        ));

    // 合并路由
    router = router.merge(authenticated_routes);

    // 添加动态路由
    let service_proxy = Arc::new(service_proxy);
    for route in &gateway_config.routes.routes {
        router = add_service_route(
            router,
            route,
            service_proxy.clone(),
            cache_instance.clone(),
            config.clone(),
        );
    }

    // 未匹配路径统一 404 + 信封：axum 默认的空 body 404 让客户端分不清
    // 「路由不存在」和「网关崩了」
    Ok(router
        .fallback(|| async {
            crate::error::GatewayError::not_found("接口不存在").into_response()
        })
        .layer(Extension(user_service))
        .layer(Extension(cache_instance.clone() as Arc<dyn SessionCache>))
        .layer(Extension(cache_instance as Arc<dyn PresenceCache>))
        .with_state(config))
}

/// 添加服务路由
fn add_service_route(
    router: Router<Arc<AppConfig>>,
    route: &RouteRule,
    service_proxy: Arc<ServiceProxy>,
    cache_instance: Arc<cache::RedisCache>,
    config: Arc<AppConfig>,
) -> Router<Arc<AppConfig>> {
    let path = route.path_prefix.clone();
    let service_type = route.service_type.clone();
    let require_auth = route.require_auth;

    // 创建处理函数的工厂函数
    //
    // 转发错误直接交给 `GatewayError::into_response`：状态码由错误本身决定
    // （400/401/404/502/503/504），响应体不含下游 Status 原文。
    let create_handler = || {
        let service_proxy = service_proxy.clone();
        let service_type = service_type.clone();
        move |req: Request<Body>| {
            let service_proxy = service_proxy.clone();
            let service_type = service_type.clone();
            async move {
                match service_proxy.forward_request(req, &service_type).await {
                    Ok(response) => response,
                    Err(err) => err.into_response(),
                }
            }
        }
    };

    info!("添加路由: {} (require_auth: {})", path, require_auth);

    // 根据认证要求添加路由
    let route_handler = if require_auth {
        any(create_handler())
            .layer(axum::Extension(cache_instance.clone()))
            .layer(middleware::from_fn_with_state(config.clone(), auth_middleware))
    } else {
        any(create_handler())
    };

    // 添加精确路径和通配符路径
    let wildcard_path = format!("{}/{{*path}}", path);
    let wildcard_handler = if require_auth {
        any(create_handler())
            .layer(axum::Extension(cache_instance.clone()))
            .layer(middleware::from_fn_with_state(config.clone(), auth_middleware))
    } else {
        any(create_handler())
    };

    router
        .route(&path, route_handler)
        .route(&wildcard_path, wildcard_handler)
}

/// 健康检查处理函数
async fn health_check() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "service": "api-gateway",
            "version": env!("CARGO_PKG_VERSION"),
            "api_documentation": {
                "swagger_ui": "/swagger-ui",
                "openapi_json": "/api-doc/openapi.json"
            }
        })),
    )
}


/// 获取上传签名
async fn get_upload_signature(
    State(config): State<Arc<AppConfig>>,
    Extension(user_info): Extension<Claims>,
    Json(req): Json<GetUploadSignatureRequest>,
) -> Result<impl IntoResponse, GatewayError> {
    // 使用注入的配置创建OSS客户端
    let oss_client = match oss(&config).await {
        Ok(client) => client,
        Err(e) => {
            error!("创建OSS客户端失败: {}", e);
            return Err(GatewayError::internal(format!("OSS 客户端初始化失败: {e}")));
        }
    };

    let bucket_type_str = req.bucket_type.as_deref().unwrap_or("file");

    // 解析bucket类型
    let bucket_type = match bucket_type_str {
        "file" => BucketType::File,
        "avatar" => BucketType::Avatar,
        _ => BucketType::File
    };

    // 生成路径前缀（前端可以在此前缀下自由上传）
    let path_prefix = match bucket_type {
        BucketType::File => format!(
            "uploads/{}/{}/{}",  // 租户ID/日期/文件ID
            user_info.tenant_id,
            chrono::Utc::now().format("%Y/%m/%d"),
            Uuid::new_v4()
        ),
        BucketType::Avatar => format!(
            "avatars/{}/{}",  // 租户ID/文件ID
            user_info.tenant_id,
            Uuid::new_v4()
        ),
    };

    // 设置过期时间
    let expiration = match bucket_type {
        BucketType::File => Duration::from_secs(3600), // 1小时
        BucketType::Avatar => Duration::from_secs(1800), // 30分钟
    };

    // 生成通用上传签名（允许上传到路径前缀下）
    let signature = match oss_client
        .generate_upload_signature(&path_prefix, req.content_type.as_str(), expiration, bucket_type)
        .await
    {
        Ok(signature) => signature,
        Err(e) => {
            error!("生成上传签名失败: {}", e);
            return Err(GatewayError::internal(format!("生成上传签名失败: {e}")));
        }
    };
    Ok(success_response(signature))
}

