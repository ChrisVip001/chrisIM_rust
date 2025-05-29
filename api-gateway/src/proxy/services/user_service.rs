use axum::{
    body::Body,
    http::{Method, Response, StatusCode},
};
use common::grpc_client::UserServiceGrpcClient;
use common::proto;
use serde_json::{json, Value};
use tracing::{error, debug, info};
use crate::proxy::services::common::get_user_id_from_jwt;
use super::common::{success_response, success_with_message, error_response, extract_string_param, get_optional_string, format_timestamp};
use crate::auth::jwt::UserInfo;

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
        &mut self,
        method: &Method,
        path: &str,
        body: Value,
        jwt_user_info: Option<UserInfo>,
    ) -> Result<Response<Body>, anyhow::Error> {
        debug!("处理用户服务请求: {} {}", method, path);

        // 从JWT中获取用户ID
        let current_user_id = get_user_id_from_jwt(jwt_user_info.as_ref())?;
        
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
                let username = extract_string_param(&body, "username", None)?;
                let password = extract_string_param(&body, "password", None)?;
                let email = get_optional_string(&body, "email", None).unwrap_or_default();
                let nickname = get_optional_string(&body, "nickname", None).unwrap_or_default();
                let avatar_url = get_optional_string(&body, "avatarUrl", Some("avatar_url")).unwrap_or_default();

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
                    StatusCode::OK
                ))
            }

            // 更新用户
            (&Method::POST, "updateUser") => {
                // userid从token中获取
                let user_id = Some(current_user_id);
                let nickname = get_optional_string(&body, "nickname", None);
                let email = get_optional_string(&body, "email", None);
                let avatar_url = get_optional_string(&body, "avatarUrl", Some("avatar_url"));
                // 密码不让在此修改
                // let password = get_optional_string(&body, "password", None);
                let password = None;
                let address = get_optional_string(&body, "address", None);
                let head_image = get_optional_string(&body, "headImage", Some("head_image"));
                let head_image_thumb = get_optional_string(&body, "headImageThumb", Some("head_image_thumb"));
                let custom_id = get_optional_string(&body, "customId", Some("custom_id"));
                let sex = get_optional_string(&body, "sex", None)
                    .and_then(|s| s.parse::<i32>().ok());
                let username = get_optional_string(&body, "username", None);

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
                    username,
                    custom_id,
                };

                let response = self.client.update_user(request).await?;
                let user = response.user.ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;

                Ok(success_with_message(
                    self.convert_user_to_json(&user),
                    "用户更新成功",
                    StatusCode::OK
                ))
            }

            // 用户账号密码注册(不校验验证码)
            (&Method::POST, "registerByUsername") => {
                let tenant_id = extract_string_param(&body,"tenantId",Some("tenant_id"))?;
                let username = extract_string_param(&body,"username",None)?;
                let password = extract_string_param(&body,"password",None)?;
                let phone = extract_string_param(&body,"phone",None)?;

                let request = proto::user::RegisterRequest {
                    tenant_id,
                    username,
                    password,
                    phone,
                    verify_code: "".to_string(),
                    nickname: "".to_string(),
                };

                match self.client.register_by_username(request).await {
                    Ok(response) => {
                        let user = response
                            .user
                            .ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;
                        Ok(success_with_message(
                            self.convert_user_to_json(&user),
                            "用户注册成功",
                            StatusCode::OK
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

                let tenant_id = extract_string_param(&body,"tenantId",Some("tenant_id"))?;
                let phone = extract_string_param(&body,"phone",None)?;
                let password = extract_string_param(&body,"password",None)?;
                let msg_code = extract_string_param(&body,"msgCode",Some("msg_code"))?;

                let request = proto::user::RegisterRequest {
                    tenant_id,
                    phone,
                    password,
                    verify_code: msg_code,
                    username: "".to_string(),
                    nickname: "".to_string(),
                };

                match self.client.register_by_phone(request).await {
                    Ok(response) => {
                        let user = response
                            .user
                            .ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;
                        Ok(success_with_message(
                            self.convert_user_to_json(&user),
                            "用户注册成功",
                            StatusCode::OK
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
                // let username = extract_string_param(&body, "username", None)?;
                let password = extract_string_param(&body, "password", None)?;
                let tenant_id = get_optional_string(&body, "tenantId", Some("tenant_id")).unwrap_or_default();
                let phone = get_optional_string(&body, "phone", None).unwrap_or_default();
                let verify_code = get_optional_string(&body, "verifyCode", Some("verify_code")).unwrap_or_default();

                let request = proto::user::ForgetPasswordRequest {
                    password: password.to_string(),
                    tenant_id: tenant_id.to_string(),
                    phone: phone.to_string(),
                    verify_code: verify_code.to_string(),
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
                info!("时间: {}", user_config.clone().create_time.unwrap_or_default());
                Ok(success_response(self.convert_user_config_to_json(&user_config), StatusCode::OK))
            }

            // 保存用户设置
            (&Method::POST, "saveUserConfig")=> {
                let user_id = extract_string_param(&body, "userId", Some("user_id"))?;
                let allow_phone_search = get_optional_string(&body, "allowPhoneSearch", Some("allow_phone_search"))
                    .and_then(|s| s.parse::<i32>().ok());
                let allow_id_search = get_optional_string(&body, "allowIdSearch", Some("allow_id_search"))
                    .and_then(|s| s.parse::<i32>().ok());
                let auto_load_video = get_optional_string(&body, "autoLoadVideo", Some("auto_load_video"))
                    .and_then(|s| s.parse::<i32>().ok());
                let auto_load_pic = get_optional_string(&body, "autoLoadPic", Some("auto_load_pic"))
                    .and_then(|s| s.parse::<i32>().ok());
                let msg_read_flag = get_optional_string(&body, "msgReadFlag", Some("msg_read_flag"))
                    .and_then(|s| s.parse::<i32>().ok());

                let request = proto::user::UserConfigRequest {
                    user_id: user_id.to_string(),
                    allow_phone_search,
                    allow_id_search,
                    auto_load_video,
                    auto_load_pic,
                    msg_read_flag,
                };
                let response = self.client.save_user_config(request).await?;
                let user_config = response.user_config.unwrap_or_default();
                Ok(success_response(self.convert_user_config_to_json(&user_config), StatusCode::OK))
            }
            
            // 发送手机验证码
            (&Method::POST, "sendVerificationCode") => {
                let phone = extract_string_param(&body, "phone", None)?;
                let action = get_optional_string(&body, "action", None).unwrap_or_default();
                
                let request = proto::user::PhoneVerificationRequest {
                    phone: phone.to_string(),
                    action: action.to_string(),
                };
                
                match self.client.send_phone_verification_code(request).await {
                    Ok(response) => {
                        if response.success {
                            Ok(success_with_message(
                                json!({}),
                                &response.message,
                                StatusCode::OK
                            ))
                        } else {
                            Ok(error_response(&response.message, StatusCode::BAD_REQUEST))
                        }
                    }
                    Err(err) => {
                        error!("发送验证码失败: {}", err);
                        Ok(error_response(&format!("发送验证码失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR))
                    }
                }
            }
            
            // 验证手机验证码
            (&Method::POST, "verifyPhoneCode") => {
                let phone = extract_string_param(&body, "phone", None)?;
                let code = extract_string_param(&body, "verifyCode", Some("verify_code"))
                    .or_else(|_| extract_string_param(&body, "msgCode", Some("msg_code")))
                    .or_else(|_| extract_string_param(&body, "code", None))?;
                let action = get_optional_string(&body, "action", None).unwrap_or("register".to_string());
                
                let request = proto::user::VerifyPhoneCodeRequest {
                    phone: phone.to_string(),
                    code: code.to_string(),
                    action: action.to_string(),
                };
                
                match self.client.verify_phone_code(request).await {
                    Ok(response) => {
                        if response.valid {
                            Ok(success_with_message(
                                json!({"valid": true}),
                                &response.message,
                                StatusCode::OK
                            ))
                        } else {
                            Ok(error_response(&response.message, StatusCode::BAD_REQUEST))
                        }
                    }
                    Err(err) => {
                        error!("验证码验证失败: {}", err);
                        Ok(error_response(&format!("验证码验证失败: {}", err), StatusCode::INTERNAL_SERVER_ERROR))
                    }
                }
            }

            //根据token获取用户信息(用户id等信息已经在jwt_user_info里了用id查找用户详细信息)
            (&Method::GET, "getUserInfo") => {
                let user_id = match &jwt_user_info {
                    Some(user_info) => {
                        user_info.user_id.to_string()
                    },
                    None => return Ok(error_response("未授权", StatusCode::UNAUTHORIZED))
                };

                let response = self.client.get_user(&user_id).await?;
                let user = response.user.ok_or_else(|| anyhow::anyhow!("用户数据为空"))?;

                Ok(success_response(self.convert_user_to_json(&user), StatusCode::OK))
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
            "host" : user.address,
            "head_image" : user.head_image,
            "head_image_thumb" : user.head_image_thumb,
            "sex" : user.sex,
            "user_stat" : user.user_stat,
            "tenant_id" : user.tenant_id,
            "last_login_time" : format_timestamp(user.last_login_time.clone()),
            "custom_id" : user.custom_id,
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