use axum::{
    body::Body,
    http::{Method, Response, StatusCode},
};
use common::grpc_client::UserServiceGrpcClient;
use common::proto;
use serde_json::{json, Value};
use tracing::{error, debug};

use super::common::{success_response, success_with_message, error_response, extract_string_param, get_optional_string, timestamp_to_rfc3339, format_timestamp};

/// 用户服务处理器
#[derive(Clone)]
pub struct UserServiceHandler {
    client: UserServiceGrpcClient,
}

impl UserServiceHandler {
    /// 创建新的用户服务处理器
    pub fn new(client: UserServiceGrpcClient) -> Self {
        Self { client }
    }

    /// 处理用户服务请求
    pub async fn handle_request(
        &self,
        method: &Method,
        path: &str,
        body: Value,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("处理用户服务请求: {} {}", method, path);

        // 从路径提取方法名 - 格式: /api/users/[method]
        let method_name = path.split('/').nth(3).unwrap_or("unknown");

        match (method, method_name) {
            // 用户查询
            (&Method::GET, "getUserById") | (&Method::GET, "getUser") => {
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let response = self.client.get_user(&user_id).await?;
                let user = response.user.ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;

                Ok(success_response(self.convert_user_to_json(&user), StatusCode::OK))
            }

            // 用户名查询
            (&Method::GET, "getUserByUsername") => {
                let username = extract_string_param(&body, "username", None)?;

                let response = self.client.get_user_by_username(&username).await?;
                let user = response.user.ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;

                Ok(success_response(self.convert_user_to_json(&user), StatusCode::OK))
            }

            // 创建用户
            (&Method::POST, "createUser") | (&Method::POST, "register") => {
                let username = body.get("username").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("用户名不能为空"))?;
                let password = body.get("password").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("密码不能为空"))?;

                if username.is_empty() || password.is_empty() {
                    return Err(anyhow::anyhow!("用户名和密码不能为空"));
                }

                let email = body.get("email").and_then(|v| v.as_str()).unwrap_or_default();
                let nickname = body.get("nickname").and_then(|v| v.as_str()).unwrap_or_default();
                let avatar_url = body.get("avatarUrl").or_else(|| body.get("avatar_url"))
                    .and_then(|v| v.as_str()).unwrap_or_default();

                let request = proto::user::CreateUserRequest {
                    username: username.to_string(),
                    email: email.to_string(),
                    password: password.to_string(),
                    nickname: nickname.to_string(),
                    avatar_url: avatar_url.to_string(),
                };

                let response = self.client.create_user(request).await?;
                let user = response.user.ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;

                Ok(success_with_message(
                    self.convert_user_to_json(&user),
                    "用户创建成功",
                    StatusCode::CREATED
                ))
            }

            // 更新用户
            (&Method::PUT, "updateUser") | (&Method::PATCH, "updateUser") => {
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;

                let nickname = get_optional_string(&body, "nickname", None);
                let email = get_optional_string(&body, "email", None);
                let avatar_url = get_optional_string(&body, "avatarUrl", Some("avatar_url"));
                let password = get_optional_string(&body, "password", None);
                let address = get_optional_string(&body, "address", None);
                let head_image = get_optional_string(&body, "head_image", None);
                let head_image_thumb = get_optional_string(&body, "head_image_thumb", None);
                let sex = get_optional_string(&body, "sex", None)
                    .and_then(|s| s.parse::<i32>().ok());

                let request = proto::user::UpdateUserRequest {
                    user_id,
                    nickname,
                    email,
                    avatar_url,
                    password,
                    address,
                    head_image,
                    head_image_thumb,
                    sex,
                };

                let response = self.client.update_user(request).await?;
                let user = response.user.ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;

                Ok(success_with_message(
                    self.convert_user_to_json(&user),
                    "用户更新成功",
                    StatusCode::OK
                ))
            }

            // 用户账号密码注册
            (&Method::POST, "registerByUsername") => {
                let username = body
                    .get("username")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let password = body
                    .get("password")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let nickname = body
                    .get("nickname")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let tenant_id = body
                    .get("tenant_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let phone = body
                    .get("phone")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let msg_code = body
                    .get("msg_code")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                if username.is_empty() || password.is_empty() {
                    return Ok(error_response("用户名或者密码不能为空", StatusCode::BAD_REQUEST));
                }

                let request = proto::user::RegisterRequest {
                    username: username.to_string(),
                    password: password.to_string(),
                    nickname: nickname.to_string(),
                    tenant_id: tenant_id.to_string(),
                    phone: phone.to_string(),
                    msg_code: msg_code.to_string(),
                };

                match self.client.register_by_username(request).await {
                    Ok(response) => {
                        let user = response
                            .user
                            .ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;
                        Ok(success_with_message(
                            self.convert_user_to_json(&user),
                            "用户注册成功",
                            StatusCode::CREATED
                        ))
                    }
                    Err(err) => {
                        error!("注册用户失败: {}", err);
                        Ok(error_response(&format!("注册用户失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR))
                    }
                }
            }

            // 用户手机号注册
            (&Method::POST, "registerByPhone") => {
                let username = body
                    .get("username")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let password = body
                    .get("password")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let nickname = body
                    .get("nickname")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let tenant_id = body
                    .get("tenant_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let phone = body
                    .get("phone")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let msg_code = body
                    .get("msg_code")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                if phone.is_empty() || password.is_empty() {
                    return Ok(error_response("手机号或者密码不能为空", StatusCode::BAD_REQUEST));
                }

                let request = proto::user::RegisterRequest {
                    username: username.to_string(),
                    password: password.to_string(),
                    nickname: nickname.to_string(),
                    tenant_id: tenant_id.to_string(),
                    phone: phone.to_string(),
                    msg_code: msg_code.to_string(),
                };

                match self.client.register_by_phone(request).await {
                    Ok(response) => {
                        let user = response
                            .user
                            .ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;
                        Ok(success_with_message(
                            self.convert_user_to_json(&user),
                            "用户注册成功",
                            StatusCode::CREATED
                        ))
                    }
                    Err(err) => {
                        error!("注册用户失败: {}", err);
                        Ok(error_response(&format!("注册用户失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR))
                    }
                }
            }

            // 忘记密码
            (&Method::POST, "forgetPassword") => {
                let username = body
                    .get("username")
                    .or_else(|| body.get("username"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let phone = body
                    .get("phone")
                    .or_else(|| body.get("phone"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                if username.is_empty() && phone.is_empty() {
                    return Ok(error_response("用户名或者手机号不能为空", StatusCode::BAD_REQUEST));
                }

                let password = body
                    .get("password")
                    .or_else(|| body.get("password"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let tenant_id = body
                    .get("tenant_id")
                    .or_else(|| body.get("tenant_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                let request = proto::user::ForgetPasswordRequest {
                    username: username.to_string(),
                    password: password.to_string(),
                    tenant_id: tenant_id.to_string(),
                    phone: phone.to_string(),
                };

                match self.client.forget_password(request).await {
                    Ok(response) => {
                        let user = response
                            .user
                            .ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;
                        Ok(success_with_message(
                            self.convert_user_to_json(&user),
                            "密码更新成功",
                            StatusCode::OK
                        ))
                    }
                    Err(err) => {
                        error!("密码更新失败: {}", err);
                        Ok(error_response(&format!("密码更新失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR))
                    }
                }
            }

            // 用户设置查询
            (&Method::GET, "getUserConfig")=> {
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let response = self.client.get_user_config(&user_id).await?;
                let user_config = response.user_config.unwrap_or_default();
                Ok(success_response(self.convert_user_config_to_json(&user_config), StatusCode::OK))
            }

            // 其他未知方法
            _ => {
                error!("未知的用户服务方法: {}", method_name);
                Err(anyhow::anyhow!("未实现的方法: {}", method_name))
            }
        }
    }

    /// 将用户消息转换为JSON
    fn convert_user_to_json(&self, user: &proto::user::User) -> Value {
        json!({
            "id": user.id,
            "username": user.username,
            "email": user.email,
            "nickname": user.nickname,
            "avatarUrl": user.avatar_url,
            "createdAt": format_timestamp(user.created_at.clone()),
            "updatedAt": format_timestamp(user.updated_at.clone()),
            "phone" : user.phone,
            "address" : user.address,
            "head_image" : user.head_image,
            "head_image_thumb" : user.head_image_thumb,
            "sex" : user.sex,
            "user_stat" : user.user_stat,
            "tenant_id" : user.tenant_id,
            "last_login_time" : format_timestamp(user.last_login_time.clone()),
            "user_idx" : user.user_idx,
        })
    }

    fn convert_user_config_to_json(&self, user_config: &proto::user::UserConfig) -> Value {
        json!({
            "user_id": user_config.user_id,
            "allow_phone_search": user_config.allow_phone_search,
            "allow_id_search": user_config.allow_id_search,
            "auto_load_video": user_config.auto_load_video,
            "auto_load_pic": user_config.auto_load_pic,
            "msg_read_flag": user_config.msg_read_flag,
            "create_time": format_timestamp(user_config.create_time.clone()),
            "update_time": format_timestamp(user_config.update_time.clone()),
        })
    }
} 