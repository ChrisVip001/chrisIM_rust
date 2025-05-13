use common::grpc_client::{GrpcClientFactory, GrpcServiceClient, UserServiceGrpcClient};
use common::proto::user::CreateUserRequest;
use std::sync::Arc;
use tokio::time::Duration;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::fmt::init();

    // 方法1: 使用工厂创建客户端
    let factory = GrpcClientFactory::from_env();
    let user_service_1 = factory.create_client("user-service");

    // 启动后台刷新任务
    // 注意：每个客户端都需要独立的刷新任务
    let user_service_1 = Arc::new(user_service_1);
    GrpcServiceClient::start_refresh_task(user_service_1.clone());

    // 方法2: 直接创建特定服务的客户端
    let user_service_2 = UserServiceGrpcClient::from_env();

    // 使用客户端调用服务
    match user_service_1.get_channel().await {
        Ok(channel) => {
            info!("成功获取用户服务gRPC通道");

            // 使用UserServiceGrpcClient包装通道
            let wrapped_client = UserServiceGrpcClient::new(
                factory.create_client_with_config(
                    "user-service",
                    Duration::from_secs(5),
                    Duration::from_secs(10),
                    50
                )
            );

            // 调用用户服务
            match wrapped_client.get_user("user_123").await {
                Ok(response) => {
                    info!("用户服务响应: {:?}", response);
                }
                Err(err) => {
                    info!("调用用户服务失败: {}", err);
                }
            }

            // 创建用户示例
            let create_request = CreateUserRequest {
                username: "new_user".to_string(),
                email: "new_user@example.com".to_string(),
                password: "password123".to_string(),
                nickname: "New User".to_string(),
                avatar_url: "https://example.com/avatar.png".to_string(),
            };

            match user_service_2.create_user(create_request).await {
                Ok(response) => {
                    info!("创建用户成功: {:?}", response);
                }
                Err(err) => {
                    info!("创建用户失败: {}", err);
                }
            }
        }
        Err(err) => {
            info!("无法获取用户服务gRPC通道: {}", err);
        }
    }

    // 等待一段时间，确保后台任务有机会运行
    tokio::time::sleep(Duration::from_secs(2)).await;

    Ok(())
}