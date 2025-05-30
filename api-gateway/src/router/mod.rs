use crate::auth::controller;
use crate::auth::middleware::auth_middleware;
use crate::proxy::ServiceProxy;
use axum::{
    body::Body,
    extract::{Json, Path, Query},
    http::{Request, StatusCode},
    middleware,
    response::IntoResponse,
    routing::{any, get, post},
    Router,
};
use common::{
    config::ConfigLoader,
    configs::{GatewayConfig, routes_config::RouteRule},
    grpc_client::{base::get_rpc_client, UserServiceGrpcClient as CommonUserServiceClient},
    proto::user::user_service_client::UserServiceClient,
    service_discovery::LbWithServiceDiscovery,
};
use oss::oss;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tracing::{error, info};
use uuid::Uuid;

// 预签名URL请求参数
#[derive(Debug, Deserialize)]
pub struct PresignedUrlRequest {
    pub filename: String,
    pub content_type: String,
    pub file_size: usize,
}

// 预签名URL响应
#[derive(Debug, Serialize)]
pub struct PresignedUrlResponse {
    pub upload_url: String,
    pub key: String,
}

// 文件上传完成验证请求
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

/// 构建路由
pub async fn build_routes(
    service_proxy: ServiceProxy,
    gateway_config: &GatewayConfig,
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
        .route("/api/user/refresh", post(controller::refresh_token))
        // 文件上传路由（无需认证）
        .route("/api/files/presigned-url", post(get_presigned_upload_url))
        .route("/api/files/validate-upload", post(validate_file_upload))
        .route("/api/files/register-avatar", post(get_register_avatar_url))
        .route("/api/files/validate-register-avatar", post(validate_register_avatar));

    // 添加动态路由
    let service_proxy = Arc::new(service_proxy);
    for route in &gateway_config.routes.routes {
        router = add_service_route(router, route, service_proxy.clone());
    }

    // 添加用户服务扩展
    Ok(router.layer(axum::Extension(user_service)))
}

/// 添加服务路由
fn add_service_route(
    router: Router,
    route: &RouteRule,
    service_proxy: Arc<ServiceProxy>,
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
        any(create_handler()).layer(middleware::from_fn(auth_middleware))
    } else {
        any(create_handler())
    };

    // 添加精确路径和通配符路径
    let wildcard_path = format!("{}/{{*path}}", path);
    let wildcard_handler = if require_auth {
        any(create_handler()).layer(middleware::from_fn(auth_middleware))
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

/// 获取预签名上传URL
async fn get_presigned_upload_url(
    Json(req): Json<PresignedUrlRequest>,
) -> impl IntoResponse {
    // 验证文件类型和大小
    if req.file_size > 100 * 1024 * 1024 {
        // 100MB 限制
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "file_too_large",
                "message": "文件大小不能超过100MB"
            })),
        );
    }

    // 生成唯一的文件键
    let file_extension = req
        .filename
        .split('.')
        .last()
        .unwrap_or("bin")
        .to_lowercase();
    let key = format!("uploads/{}/{}.{}", 
        chrono::Utc::now().format("%Y/%m/%d"),
        Uuid::new_v4(),
        file_extension
    );

    // 获取配置并创建OSS客户端
    let config = ConfigLoader::get_global().expect("无法加载全局配置");

    let oss_client = oss(&config).await;

    // 生成预签名URL
    match oss_client.generate_presigned_upload_url(&key, &req.content_type, Duration::from_secs(3600))
        .await
    {
        Ok(upload_url) => (
            StatusCode::OK,
            Json(json!({ "key": key, "upload_url": upload_url })),
        ),
        Err(e) => {
            error!("生成预签名URL失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "presigned_url_generation_failed",
                    "message": "无法生成上传URL"
                })),
            )
        }
    }
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
