use crate::auth::controller;
use crate::auth::middleware::auth_middleware;
use crate::proxy::ServiceProxy;
use axum::{
    body::Body,
    extract::{Json, Extension},
    http::{Request, Response, StatusCode},
    middleware,
    response::IntoResponse,
    routing::{any, get, post},
    Router,
};
use common::{config::ConfigLoader, configs::{GatewayConfig, routes_config::RouteRule}, grpc_client::{base::get_rpc_client}, proto::user::user_service_client::UserServiceClient, service_discovery::LbWithServiceDiscovery, Error};
use oss::{oss, BucketType};
use serde::{Deserialize};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tracing::{error, info};
use uuid::Uuid;
use common::auth::Claims;
use crate::proxy::services::common::{error_response, success_response};

// 获取上传签名请求参数
#[derive(Debug, Deserialize)]
pub struct GetUploadSignatureRequest {
    pub content_type: String,
    pub bucket_type: Option<String>, // "file" 或 "avatar"，默认"file"
}

// 文件上传完成验证请求（保留用于验证）
#[derive(Debug, Deserialize)]
pub struct ValidateUploadRequest {
    pub key: String,
    pub size: usize,
    pub md5: String,
}

// 注册头像上传请求
#[derive(Debug, Deserialize)]
pub struct RegisterAvatarRequest {
    pub content_type: String,
    pub file_size: usize,
}

// 验证注册头像上传请求
#[derive(Debug, Deserialize)]
pub struct ValidateRegisterAvatarRequest {
    pub key: String,
    pub size: usize,
    pub md5: String,
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
    gateway_config: &GatewayConfig,
    cache_instance: Arc<dyn cache::Cache>,
) -> anyhow::Result<Router> {
    // 创建用户服务客户端
    let config = ConfigLoader::get_global().expect("获取配置失败");
    let service_client = get_rpc_client::<UserServiceClient<LbWithServiceDiscovery>>(
        &config,
        "user".to_string(),
    )
    .await?;

    let user_service = controller::SharedUserService::new(service_client);

    // 构建基础路由
    let mut router = Router::new()
        // 健康检查和指标
        .route("/health", get(health_check))
        .route(&gateway_config.metrics_endpoint, get(crate::metrics::get_metrics_handler))
        // 认证路由（无需认证）
        .route("/api/user/login", post(controller::login))
        .route("/api/user/loginByPhone", post(controller::login_by_phone))
        .route("/api/user/refresh", post(controller::refresh_token));

    // 需要认证的路由
    let authenticated_routes = Router::new()
        // 用户管理路由
        .route("/api/user/logout", post(controller::logout))
        // 好友在线状态查询路由
        .route("/api/friends/online-status", post(controller::batch_get_friends_online_status))
        // 文件上传签名路由（需要认证以获取租户ID）
        .route("/api/files/upload-signature", post(get_upload_signature))
        .layer(Extension(cache_instance.clone()))
        .layer(middleware::from_fn(auth_middleware));

    // 合并路由
    router = router.merge(authenticated_routes);

    // 添加动态路由
    let service_proxy = Arc::new(service_proxy);
    for route in &gateway_config.routes.routes {
        router = add_service_route(router, route, service_proxy.clone(), cache_instance.clone());
    }

    // 添加用户服务扩展和缓存扩展
    Ok(router
        .layer(Extension(user_service))
        .layer(Extension(cache_instance)))
}

/// 添加服务路由
fn add_service_route(
    router: Router,
    route: &RouteRule,
    service_proxy: Arc<ServiceProxy>,
    cache_instance: Arc<dyn cache::Cache>,
) -> Router {
    let path = route.path_prefix.clone();
    let service_type = route.service_type.clone();
    let require_auth = route.require_auth;

    // 创建处理函数的工厂函数
    let create_handler = || {
        let service_proxy = service_proxy.clone();
        let service_type = service_type.clone();
        move |req: Request<Body>| {
            let service_proxy = service_proxy.clone();
            let service_type = service_type.clone();
            async move { service_proxy.forward_request(req, &service_type).await.unwrap_or_else(
                |err| {
                    error_response(&format!("转发请求到后端服务失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR)
                },
            ) }
        }
    };

    info!("添加路由: {} (require_auth: {})", path, require_auth);

    // 根据认证要求添加路由
    let route_handler = if require_auth {
        any(create_handler())
            .layer(axum::Extension(cache_instance.clone()))
            .layer(middleware::from_fn(auth_middleware))
    } else {
        any(create_handler())
    };

    // 添加精确路径和通配符路径
    let wildcard_path = format!("{}/{{*path}}", path);
    let wildcard_handler = if require_auth {
        any(create_handler())
            .layer(axum::Extension(cache_instance.clone()))
            .layer(middleware::from_fn(auth_middleware))
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
    Extension(user_info): Extension<Claims>,
    Json(req): Json<GetUploadSignatureRequest>,
) -> Result<impl IntoResponse, Error> {
    // 获取配置并创建OSS客户端
    let config = ConfigLoader::get_global().expect("无法加载全局配置");
    let oss_client = match oss(&config).await {
        Ok(client) => client,
        Err(e) => {
            error!("创建OSS客户端失败: {}", e);
            return Ok(error_response(
                &format!("OSS服务初始化失败: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            ));
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
            return Ok(error_response(
                &format!("生成上传签名失败: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            ));
        }
    };
    Ok(success_response(signature, StatusCode::OK))
}

