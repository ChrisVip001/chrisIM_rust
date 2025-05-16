use anyhow::Result;
use std::sync::Arc;
use common::grpc_client::GrpcServiceClient;
use common::proto::user::{GetUserByIdRequest, UserResponse, CreateUserRequest};
use tracing::info;

// 导入宏
use common::generate_grpc_client;
use common::simple_grpc_client;

// 使用宏自动生成用户服务客户端
generate_grpc_client!(
    name: UserServiceGrpcClientGen, 
    service: "user-service",
    proto_path: common::proto::user,
    client_type: user_service_client::UserServiceClient,
    methods: [
        get_user_by_id(GetUserByIdRequest) -> UserResponse,
        get_user_by_username(GetUserByUsernameRequest) -> UserResponse,
        create_user(CreateUserRequest) -> UserResponse,
        update_user(UpdateUserRequest) -> UserResponse,
    ]
);

// 使用简化版宏生成好友服务客户端
simple_grpc_client!(
    FriendServiceGrpcClientGen, 
    "friend-service",
    common::proto::friend,
    friend_service_client::FriendServiceClient
);

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    tracing_subscriber::fmt::init();

    // 创建用户服务客户端
    let user_client = UserServiceGrpcClientGen::from_env();

    // 查询用户
    match user_client.get_user_by_id(GetUserByIdRequest {
        user_id: "user_123".to_string(),
    }).await {
        Ok(response) => {
            info!("用户查询成功: {:?}", response);
        }
        Err(err) => {
            info!("用户查询失败: {}", err);
        }
    }

    // 创建用户
    let create_request = CreateUserRequest {
        username: "new_user".to_string(),
        email: "new_user@example.com".to_string(),
        password: "password123".to_string(),
        nickname: "New User".to_string(),
        avatar_url: "https://example.com/avatar.png".to_string(),
    };

    match user_client.create_user(create_request).await {
        Ok(response) => {
            info!("创建用户成功: {:?}", response);
        }
        Err(err) => {
            info!("创建用户失败: {}", err);
        }
    }

    // 创建简化版好友服务客户端
    let friend_client = FriendServiceGrpcClientGen::from_env();
    let mut friend_grpc_client = match friend_client.get_client().await {
        Ok(client) => {
            info!("成功获取好友服务客户端");
            client
        }
        Err(err) => {
            info!("获取好友服务客户端失败: {}", err);
            return Ok(());
        }
    };

    // 对于简化版客户端，使用其基础方法，但需要手动实现具体方法调用
    // 这里演示如何手动调用查询好友列表方法
    use common::proto::friend::{GetFriendListRequest, GetFriendListResponse};
    use tonic::Request;

    let friend_request = Request::new(GetFriendListRequest {
        user_id: "user_123".to_string(),
    });

    match friend_grpc_client.get_friend_list(friend_request).await {
        Ok(response) => {
            let response_inner = response.into_inner();
            info!("好友列表查询成功，好友数量: {}", response_inner.friends.len());
        }
        Err(err) => {
            info!("好友列表查询失败: {}", err);
        }
    }

    Ok(())
} 