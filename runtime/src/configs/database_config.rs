use serde::Deserialize;

fn default_pool_size() -> u32 {
    32
}

#[derive(Debug, Deserialize, Clone)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
    /// 每个服务进程独立持有的连接池上限
    ///
    /// `PgPool::connect` 默认只有 10 条连接，消费端按会话分组并行时
    /// 会被连接池排队，成为写入吞吐的硬上限。
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,
}

impl PostgresConfig {
    /// 连接池大小限幅 [4, 64]，防止配置误填把 PG 连接数打满
    pub fn effective_pool_size(&self) -> u32 {
        self.pool_size.clamp(4, 64)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub postgres: PostgresConfig,
    pub xdb: String,
}

impl DatabaseConfig {
    pub fn pg_url(&self) -> String {
        format!(
            "postgres://{}:{}@{}:{}/{}",
            self.postgres.user,
            self.postgres.password,
            self.postgres.host,
            self.postgres.port,
            self.postgres.database
        )
    }
}
