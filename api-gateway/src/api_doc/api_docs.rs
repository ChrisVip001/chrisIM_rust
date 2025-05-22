use utoipa::OpenApi;
use axum::{Router, routing::get};
use utoipa::Modify;
use tracing::info;
use serde_json;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        // 添加安全方案
        let components = openapi.components.get_or_insert_with(Default::default);
        let security_scheme = utoipa::openapi::security::SecurityScheme::ApiKey(
            utoipa::openapi::security::ApiKey::Header(
                utoipa::openapi::security::ApiKeyValue::new("Authorization")
            )
        );
        
        components.security_schemes.insert("bearer".to_string(), security_scheme);
    }
}

/// API文档配置
#[derive(OpenApi)]
#[openapi(
    paths(
        health,
        user_login,
        user_refresh,
        user_register,
        get_user_by_id,
        get_user_by_username,
        update_user,
        search_users,
        send_friend_request,
        accept_friend_request,
        reject_friend_request,
        get_friend_list,
        get_friend_requests,
        delete_friend,
        check_friendship,
        create_group,
        update_group,
        get_group,
        join_group,
        leave_group,
        list_groups,
        list_group_members
    ),
    components(
        schemas(
            HealthResponse,
            LoginRequest,
            LoginResponse,
            RefreshTokenRequest,
            RefreshTokenResponse,
            RegisterRequest,
            UserResponse,
            UpdateUserRequest,
            SearchUsersRequest,
            SearchUsersResponse,
            FriendRequest,
            FriendResponse,
            FriendListResponse,
            FriendRequestsResponse,
            DeleteFriendRequest,
            CheckFriendshipResponse,
            GroupResponse,
            CreateGroupRequest,
            UpdateGroupRequest,
            GroupListResponse,
            GroupMembersResponse
        )
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "health", description = "健康检查接口"),
        (name = "auth", description = "用户认证接口"),
        (name = "users", description = "用户管理接口"),
        (name = "friends", description = "好友管理接口"),
        (name = "groups", description = "群组管理接口"),
        (name = "messages", description = "消息管理接口")
    ),
    info(
        title = "RustIM API",
        version = "1.0.0",
        description = "RustIM 系统的API接口文档 - 一个基于Rust开发的即时通讯系统",
        contact(
            name = "RustIM 团队",
            email = "contact@rustim.example.com",
            url = "https://github.com/yourusername/rustIM_demo"
        ),
        license(
            name = "MIT",
            url = "https://opensource.org/licenses/MIT"
        )
    ),
    security(
        ("bearer" = [])
    )
)]
pub struct ApiDoc;

/// 健康检查响应
#[derive(utoipa::ToSchema, serde::Serialize)]
pub struct HealthResponse {
    status: String,
    version: String,
}

/// 登录请求
#[derive(utoipa::ToSchema)]
pub struct LoginRequest {
    username: String,
    password: String,
}

/// 登录响应
#[derive(utoipa::ToSchema)]
pub struct LoginResponse {
    token: String,
    refresh_token: String,
    expires_in: u64,
    user_id: String,
}

/// 刷新令牌请求
#[derive(utoipa::ToSchema)]
pub struct RefreshTokenRequest {
    refresh_token: String,
}

/// 刷新令牌响应
#[derive(utoipa::ToSchema)]
pub struct RefreshTokenResponse {
    token: String,
    refresh_token: String,
    expires_in: u64,
}

/// 注册请求
#[derive(utoipa::ToSchema)]
pub struct RegisterRequest {
    username: String,
    password: String,
    email: String,
    nickname: String,
}

/// 用户响应
#[derive(utoipa::ToSchema)]
pub struct UserResponse {
    id: String,
    username: String,
    nickname: String,
    email: String,
    avatar: Option<String>,
    created_at: String,
    updated_at: String,
}

/// 更新用户请求
#[derive(utoipa::ToSchema)]
pub struct UpdateUserRequest {
    nickname: Option<String>,
    email: Option<String>,
    avatar: Option<String>,
    password: Option<String>,
}

/// 搜索用户请求
#[derive(utoipa::ToSchema)]
pub struct SearchUsersRequest {
    query: String,
    page: i32,
    page_size: i32,
}

/// 搜索用户响应
#[derive(utoipa::ToSchema)]
pub struct SearchUsersResponse {
    users: Vec<UserResponse>,
    total: i64,
}

/// 好友请求
#[derive(utoipa::ToSchema)]
pub struct FriendRequest {
    user_id: String,
    friend_id: String,
    message: Option<String>,
}

/// 好友响应
#[derive(utoipa::ToSchema)]
pub struct FriendResponse {
    id: String,
    user_id: String,
    friend_id: String,
    status: i32,
    created_at: String,
    updated_at: String,
}

/// 好友列表响应
#[derive(utoipa::ToSchema)]
pub struct FriendListResponse {
    friends: Vec<FriendResponse>,
}

/// 好友请求列表响应
#[derive(utoipa::ToSchema)]
pub struct FriendRequestsResponse {
    requests: Vec<FriendResponse>,
}

/// 删除好友请求
#[derive(utoipa::ToSchema)]
pub struct DeleteFriendRequest {
    user_id: String,
    friend_id: String,
}

/// 检查好友关系响应
#[derive(utoipa::ToSchema)]
pub struct CheckFriendshipResponse {
    status: i32,
}

/// 群组响应
#[derive(utoipa::ToSchema)]
pub struct GroupResponse {
    id: String,
    name: String,
    avatar: Option<String>,
    description: Option<String>,
    owner_id: String,
    created_at: String,
    updated_at: String,
}

/// 创建群组请求
#[derive(utoipa::ToSchema)]
pub struct CreateGroupRequest {
    name: String,
    avatar: Option<String>,
    description: Option<String>,
}

/// 更新群组请求
#[derive(utoipa::ToSchema)]
pub struct UpdateGroupRequest {
    name: Option<String>,
    avatar: Option<String>,
    description: Option<String>,
}

/// 群组列表响应
#[derive(utoipa::ToSchema)]
pub struct GroupListResponse {
    groups: Vec<GroupResponse>,
}

/// 群组成员列表响应
#[derive(utoipa::ToSchema)]
pub struct GroupMembersResponse {
    members: Vec<UserResponse>,
}

/// 健康检查接口
#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    responses(
        (status = 200, description = "健康检查成功", body = HealthResponse)
    )
)]

async fn health() -> axum::Json<HealthResponse> {
    axum::Json(HealthResponse {
        status: "OK".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// 用户登录接口
#[utoipa::path(
    post,
    path = "/api/user/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "登录成功", body = LoginResponse),
        (status = 400, description = "请求参数错误"),
        (status = 401, description = "用户名或密码错误")
    )
)]
async fn user_login() {}

/// 刷新令牌接口
#[utoipa::path(
    post,
    path = "/api/user/refresh",
    tag = "auth",
    request_body = RefreshTokenRequest,
    responses(
        (status = 200, description = "刷新成功", body = RefreshTokenResponse),
        (status = 400, description = "请求参数错误"),
        (status = 401, description = "刷新令牌无效或已过期")
    )
)]
async fn user_refresh() {}

/// 用户注册接口
#[utoipa::path(
    post,
    path = "/api/user/register",
    tag = "auth",
    request_body = RegisterRequest,
    responses(
        (status = 200, description = "注册成功", body = LoginResponse),
        (status = 400, description = "请求参数错误"),
        (status = 409, description = "用户名或邮箱已存在")
    )
)]
async fn user_register() {}

/// 通过ID获取用户
#[utoipa::path(
    get,
    path = "/api/users/{user_id}",
    tag = "users",
    params(
        ("user_id" = String, Path, description = "用户ID")
    ),
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "获取用户成功", body = UserResponse),
        (status = 404, description = "用户不存在"),
        (status = 401, description = "未认证")
    )
)]
async fn get_user_by_id() {}

/// 通过用户名获取用户
#[utoipa::path(
    get,
    path = "/api/users/by-username/{username}",
    tag = "users",
    params(
        ("username" = String, Path, description = "用户名")
    ),
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "获取用户成功", body = UserResponse),
        (status = 404, description = "用户不存在"),
        (status = 401, description = "未认证")
    )
)]
async fn get_user_by_username() {}

/// 更新用户信息
#[utoipa::path(
    put,
    path = "/api/users/{user_id}",
    tag = "users",
    params(
        ("user_id" = String, Path, description = "用户ID")
    ),
    request_body = UpdateUserRequest,
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "更新用户成功", body = UserResponse),
        (status = 400, description = "请求参数错误"),
        (status = 404, description = "用户不存在"),
        (status = 401, description = "未认证")
    )
)]
async fn update_user() {}

/// 搜索用户
#[utoipa::path(
    get,
    path = "/api/users/search",
    tag = "users",
    params(
        ("query" = String, Query, description = "搜索关键词"),
        ("page" = i32, Query, description = "页码，默认为1"),
        ("page_size" = i32, Query, description = "每页数量，默认为10")
    ),
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "搜索用户成功", body = SearchUsersResponse),
        (status = 400, description = "请求参数错误"),
        (status = 401, description = "未认证")
    )
)]
async fn search_users() {}

/// 发送好友请求
#[utoipa::path(
    post,
    path = "/api/friends/request",
    tag = "friends",
    request_body = FriendRequest,
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "发送好友请求成功", body = FriendResponse),
        (status = 400, description = "请求参数错误"),
        (status = 409, description = "已经存在好友关系"),
        (status = 401, description = "未认证")
    )
)]
async fn send_friend_request() {}

/// 接受好友请求
#[utoipa::path(
    post,
    path = "/api/friends/accept",
    tag = "friends",
    request_body = FriendRequest,
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "接受好友请求成功", body = FriendResponse),
        (status = 400, description = "请求参数错误"),
        (status = 404, description = "好友请求不存在"),
        (status = 401, description = "未认证")
    )
)]
async fn accept_friend_request() {}

/// 拒绝好友请求
#[utoipa::path(
    post,
    path = "/api/friends/reject",
    tag = "friends",
    request_body = FriendRequest,
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "拒绝好友请求成功", body = FriendResponse),
        (status = 400, description = "请求参数错误"),
        (status = 404, description = "好友请求不存在"),
        (status = 401, description = "未认证")
    )
)]
async fn reject_friend_request() {}

/// 获取好友列表
#[utoipa::path(
    get,
    path = "/api/friends/list/{user_id}",
    tag = "friends",
    params(
        ("user_id" = String, Path, description = "用户ID")
    ),
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "获取好友列表成功", body = FriendListResponse),
        (status = 401, description = "未认证")
    )
)]
async fn get_friend_list() {}

/// 获取好友请求列表
#[utoipa::path(
    get,
    path = "/api/friends/requests/{user_id}",
    tag = "friends",
    params(
        ("user_id" = String, Path, description = "用户ID")
    ),
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "获取好友请求列表成功", body = FriendRequestsResponse),
        (status = 401, description = "未认证")
    )
)]
async fn get_friend_requests() {}

/// 删除好友
#[utoipa::path(
    delete,
    path = "/api/friends/{user_id}/{friend_id}",
    tag = "friends",
    params(
        ("user_id" = String, Path, description = "用户ID"),
        ("friend_id" = String, Path, description = "好友ID")
    ),
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "删除好友成功"),
        (status = 404, description = "好友关系不存在"),
        (status = 401, description = "未认证")
    )
)]
async fn delete_friend() {}

/// 检查好友关系
#[utoipa::path(
    get,
    path = "/api/friends/check/{user_id}/{friend_id}",
    tag = "friends",
    params(
        ("user_id" = String, Path, description = "用户ID"),
        ("friend_id" = String, Path, description = "好友ID")
    ),
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "检查好友关系成功", body = CheckFriendshipResponse),
        (status = 401, description = "未认证")
    )
)]
async fn check_friendship() {}

/// 创建群组
#[utoipa::path(
    post,
    path = "/api/groups",
    tag = "groups",
    request_body = CreateGroupRequest,
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "创建群组成功", body = GroupResponse),
        (status = 400, description = "请求参数错误"),
        (status = 401, description = "未认证")
    )
)]
async fn create_group() {}

/// 更新群组
#[utoipa::path(
    put,
    path = "/api/groups/{group_id}",
    tag = "groups",
    params(
        ("group_id" = String, Path, description = "群组ID")
    ),
    request_body = UpdateGroupRequest,
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "更新群组成功", body = GroupResponse),
        (status = 400, description = "请求参数错误"),
        (status = 404, description = "群组不存在"),
        (status = 403, description = "无权限更新"),
        (status = 401, description = "未认证")
    )
)]
async fn update_group() {}

/// 获取群组信息
#[utoipa::path(
    get,
    path = "/api/groups/{group_id}",
    tag = "groups",
    params(
        ("group_id" = String, Path, description = "群组ID")
    ),
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "获取群组成功", body = GroupResponse),
        (status = 404, description = "群组不存在"),
        (status = 401, description = "未认证")
    )
)]
async fn get_group() {}

/// 加入群组
#[utoipa::path(
    post,
    path = "/api/groups/{group_id}/join",
    tag = "groups",
    params(
        ("group_id" = String, Path, description = "群组ID")
    ),
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "加入群组成功"),
        (status = 404, description = "群组不存在"),
        (status = 409, description = "已经是群组成员"),
        (status = 401, description = "未认证")
    )
)]
async fn join_group() {}

/// 退出群组
#[utoipa::path(
    post,
    path = "/api/groups/{group_id}/leave",
    tag = "groups",
    params(
        ("group_id" = String, Path, description = "群组ID")
    ),
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "退出群组成功"),
        (status = 404, description = "群组不存在或不是群组成员"),
        (status = 403, description = "群主不能退出群组"),
        (status = 401, description = "未认证")
    )
)]
async fn leave_group() {}

/// 获取用户所在的群组列表
#[utoipa::path(
    get,
    path = "/api/groups/user/{user_id}",
    tag = "groups",
    params(
        ("user_id" = String, Path, description = "用户ID")
    ),
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "获取群组列表成功", body = GroupListResponse),
        (status = 401, description = "未认证")
    )
)]
async fn list_groups() {}

/// 获取群组成员列表
#[utoipa::path(
    get,
    path = "/api/groups/{group_id}/members",
    tag = "groups",
    params(
        ("group_id" = String, Path, description = "群组ID")
    ),
    security(
        ("bearer" = [])
    ),
    responses(
        (status = 200, description = "获取群组成员列表成功", body = GroupMembersResponse),
        (status = 404, description = "群组不存在"),
        (status = 401, description = "未认证")
    )
)]
async fn list_group_members() {}

/// 将API文档路由添加到Router中
pub fn configure_docs(app: Router) -> Router {
    // 日志输出API文档访问地址
    info!("API文档地址:");
    info!("- Swagger UI: /swagger-ui");
    info!("- OpenAPI JSON: /api-doc/openapi.json");
    
    // 创建OpenAPI JSON路由
    let app = app.route("/api-doc/openapi.json", get(|| async { 
        axum::Json(ApiDoc::openapi()) 
    }));
    
    // 添加API文档健康检查
    let app = app.route("/api-doc/health", get(|| async {
       info!("API文档健康检查");
       axum::Json(serde_json::json!({
           "status": "ok",
           "documentation": "API documentation is available at /swagger-ui",
           "openapi_json": "/api-doc/openapi.json",
           "version": env!("CARGO_PKG_VERSION")
       }))
    }));
    
    // 添加SwaggerUI路由，直接返回HTML内容
    app.route("/swagger-ui", get(|| async {
        let swagger_html = r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>RustIM API 文档</title>
    <link rel="stylesheet" type="text/css" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css" />
    <link rel="icon" type="image/png" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/favicon-32x32.png" sizes="32x32" />
    <link rel="icon" type="image/png" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/favicon-16x16.png" sizes="16x16" />
    <style>
        html {
            box-sizing: border-box;
            overflow: -moz-scrollbars-vertical;
            overflow-y: scroll;
        }
        
        *,
        *:before,
        *:after {
            box-sizing: inherit;
        }
        
        body {
            margin: 0;
            background: #fafafa;
        }
    </style>
</head>
<body>
    <div id="swagger-ui"></div>
    <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js" charset="UTF-8"></script>
    <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-standalone-preset.js" charset="UTF-8"></script>
    <script>
        window.onload = function() {
            const ui = SwaggerUIBundle({
                pg_url: "/api-doc/openapi.json",
                dom_id: '#swagger-ui',
                deepLinking: true,
                presets: [
                    SwaggerUIBundle.presets.apis,
                    SwaggerUIStandalonePreset
                ],
                plugins: [
                    SwaggerUIBundle.plugins.DownloadUrl
                ],
                layout: "StandaloneLayout"
            });
            window.ui = ui;
        };
    </script>
</body>
</html>
"#;
        axum::response::Html(swagger_html)
    }))
}