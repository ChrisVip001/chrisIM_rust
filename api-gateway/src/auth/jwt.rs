// JWT 功能统一管理
// 
// 所有 JWT 相关功能已移动到 common::auth 模块中，
// 这里只是重新导出以保持 API 兼容性。

pub use common::auth::{
    Claims,
    UserInfo,
    extract_token,
    verify_token,
    generate_token,
    generate_refresh_token,
    extract_user_id,
    extract_claims,
    is_token_expiring_soon,
};
