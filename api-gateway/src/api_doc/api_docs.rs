use utoipa::OpenApi;
use axum::{Router, response::IntoResponse};
use axum::routing::get;
use serde_json::json;
use std::sync::Arc;

/// API文档配置
#[derive(OpenApi)]
#[openapi(
    info(
        title = "RustIM API",
        version = "1.0.0",
        description = "RustIM 系统的API接口文档",
        contact(
            name = "RustIM 团队",
            email = "contact@rustim.example.com",
            url = "https://github.com/yourusername/rustIM_demo"
        ),
        license(
            name = "MIT",
            url = "https://opensource.org/licenses/MIT"
        )
    )
)]
pub struct ApiDoc;

/// 健康检查处理函数
async fn health_handler() -> impl IntoResponse {
    let response = json!({
        "status": "OK",
        "version": env!("CARGO_PKG_VERSION"),
        "docs": {
            "openapi": "/api-doc/openapi.json",
            "grpc_docs": "运行 ./scripts/serve-docs.sh 查看gRPC文档"
        }
    });
    response.to_string()
}

/// 将API文档路由添加到Router中
pub fn configure_docs(app: Router) -> Router {
    // 生成OpenAPI文档
    let openapi_json = serde_json::to_string_pretty(&ApiDoc::openapi()).unwrap();
    let openapi_json = Arc::new(openapi_json);

    // OpenAPI文档处理函数
    async fn openapi_handler(openapi: Arc<String>) -> impl IntoResponse {
        openapi.to_string()
    }

    let openapi_json_clone = openapi_json.clone();
    let openapi_route = get(move || async move {
        openapi_handler(openapi_json_clone.clone()).await
    });

    // 添加健康检查和OpenAPI文档端点
    app.route("/api-doc/health", get(health_handler))
        .route("/api-doc/openapi.json", openapi_route)
}