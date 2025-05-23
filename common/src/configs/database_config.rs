use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MongodbConfig {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
    pub database: String,
    pub clean: MongodbCleanConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MongodbCleanConfig {
    pub period: i64,
    pub except_types: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub postgres: PostgresConfig,
    pub mongodb: MongodbConfig,
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
    pub fn mongo_url(&self) -> String {
        format!(
            "mongodb://{:?}:{:?}@{}:{}/{}",
            self.mongodb.user,
            self.mongodb.password,
            self.mongodb.host,
            self.mongodb.port,
            self.mongodb.database
        )
    }
}
