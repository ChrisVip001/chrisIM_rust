use crate::auth::controller;
use crate::auth::middleware::auth_middleware;
use crate::proxy::ServiceProxy;
use axum::{
    body::Body,
    extract::{Json, Extension},
    http::{Request, StatusCode},
    middleware,
    response::IntoResponse,
    routing::{any, get, post},
    Router,
};
use common::{
    config::ConfigLoader,
    configs::{GatewayConfig, routes_config::RouteRule},
    grpc_client::{base::get_rpc_client},
    proto::user::user_service_client::UserServiceClient,
    service_discovery::LbWithServiceDiscovery,
};
use oss::{oss, BucketType};
use serde::{Deserialize};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tracing::{error, info};
use uuid::Uuid;
use crate::auth::jwt::UserInfo;

// 获取上传签名请求参数
#[derive(Debug, Deserialize)]
pub struct GetUploadSignatureRequest {
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
    cache_instance: std::sync::Arc<dyn cache::Cache>,
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
        .route("/api/user/logout-all", post(controller::logout_all_platforms))
        .route("/api/user/platforms", get(controller::get_user_platforms))
        // 好友在线状态查询路由
        .route("/api/friends/online-status", post(controller::batch_get_friends_online_status))
        .route("/api/friends/online-check", post(controller::batch_check_friends_online))
        // 文件上传签名路由（需要认证以获取租户ID）
        .route("/api/files/upload-signature", post(get_upload_signature))
        // 文件上传路由（无需认证）
        .route("/api/files/validate-upload", post(validate_file_upload))
        .route("/api/files/register-avatar", post(get_register_avatar_url))
        .route("/api/files/validate-register-avatar", post(validate_register_avatar))
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
        .layer(axum::Extension(user_service))
        .layer(axum::Extension(cache_instance)))
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
            async move { service_proxy.forward_request(req, &service_type).await }
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

/// 验证文件上传
async fn validate_file_upload(
    Json(req): Json<ValidateUploadRequest>,
) -> impl IntoResponse {
    // 获取配置并创建OSS客户端
    let config = ConfigLoader::get_global().expect("无法加载全局配置");

    let oss_client = oss(&config).await;

    // 验证上传
    match oss_client.validate_upload(&req.key, req.size, &req.md5).await {
        Ok(true) => (
            StatusCode::OK,
            Json(json!({
                "status": "success",
                "key": req.key,
                "is_valid": true
            })),
        ),
        Ok(false) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "validation_failed",
                "message": "文件验证失败，可能是大小或MD5不匹配"
            })),
        ),
        Err(e) => {
            error!("验证文件上传失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "validation_error",
                    "message": "无法验证文件上传"
                })),
            )
        }
    }
}

/// 获取上传签名
async fn get_upload_signature(
    Extension(user_info): Extension<UserInfo>,
    Json(req): Json<GetUploadSignatureRequest>,
) -> impl IntoResponse {
    // 获取配置并创建OSS客户端
    let config = ConfigLoader::get_global().expect("无法加载全局配置");
    let oss_client = oss(&config).await;

    let bucket_type_str = req.bucket_type.as_deref().unwrap_or("file");

    // 解析bucket类型
    let bucket_type = match bucket_type_str {
        "file" => BucketType::File,
        "avatar" => BucketType::Avatar,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "invalid_bucket_type",
                    "message": "bucket_type必须是'file'或'avatar'"
                })),
            );
        }
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
    match oss_client
        .generate_upload_signature(&path_prefix, "application/octet-stream", expiration, bucket_type)
        .await
    {
        Ok(signature) => {
            info!("为租户 {} 生成上传签名，路径前缀: {}", user_info.tenant_id, path_prefix);
            
            (
                StatusCode::OK,
                Json(json!({
                    "signature": signature
                })),
            )
        }
        Err(e) => {
            error!("生成上传签名失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "signature_generation_failed",
                    "message": "无法生成上传签名"
                })),
            )
        }
    }
}

/// 获取用于注册的头像上传URL（无需Token认证）
async fn get_register_avatar_url(
    Json(req): Json<RegisterAvatarRequest>,
) -> impl IntoResponse {
    // 验证头像文件类型
    if !is_valid_avatar_type(&req.content_type) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "invalid_content_type",
                "message": "不支持的头像文件类型，仅支持 JPEG, PNG, WebP"
            })),
        );
    }

    // 验证文件大小（头像限制为5MB）
    if req.file_size > 5 * 1024 * 1024 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "file_too_large",
                "message": "头像文件大小不能超过5MB"
            })),
        );
    }

    // 生成头像文件键
    let file_extension = match req.content_type.as_str() {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        _ => "jpg", // 默认
    };

    let key = format!(
        "avatars/register/{}.{}",
        Uuid::new_v4(),
        file_extension
    );

    // 获取配置并创建OSS客户端
    let config = ConfigLoader::get_global().expect("无法加载全局配置");

    let oss_client = oss(&config).await;

    // 生成预签名URL
    match oss_client.generate_presigned_upload_url(&key, &req.content_type, Duration::from_secs(1800))
        .await
    {
        Ok(upload_url) => (
            StatusCode::OK,
            Json(json!({ "key": key, "upload_url": upload_url })),
        ),
        Err(e) => {
            error!("生成注册头像预签名URL失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "presigned_url_generation_failed",
                    "message": "无法生成头像上传URL"
                })),
            )
        }
    }
}

/// 验证注册头像上传
async fn validate_register_avatar(
    Json(req): Json<ValidateRegisterAvatarRequest>,
) -> impl IntoResponse {
    // 获取配置并创建OSS客户端
    let config = ConfigLoader::get_global().expect("无法加载全局配置");

    let oss_client = oss(&config).await;

    // 验证上传
    match oss_client.validate_upload(&req.key, req.size, &req.md5).await {
        Ok(true) => (
            StatusCode::OK,
            Json(json!({
                "status": "success",
                "key": req.key,
                "is_valid": true
            })),
        ),
        Ok(false) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "validation_failed",
                "message": "头像文件验证失败，可能是大小或MD5不匹配"
            })),
        ),
        Err(e) => {
            error!("验证注册头像上传失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "validation_error",
                    "message": "无法验证头像上传"
                })),
            )
        }
    }
}

/// 检查头像文件类型是否有效
fn is_valid_avatar_type(content_type: &str) -> bool {
    matches!(content_type, "image/jpeg" | "image/png" | "image/webp")
}
