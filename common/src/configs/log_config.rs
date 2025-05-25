use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct LogConfig {
    pub level: String,
    pub output: Option<String>,  // 修改为可选字段
    pub sqlx_level: Option<String>,    // SQL查询日志级别
    pub components: Option<std::collections::HashMap<String, String>>, // 其他组件的日志级别
    pub format: Option<String>,        // 日志输出格式: plain或json
}

impl LogConfig {
    pub fn level(&self) -> tracing::Level {
        match self.level.as_str() {
            "trace" => tracing::Level::TRACE,
            "debug" => tracing::Level::DEBUG,
            "info" => tracing::Level::INFO,
            "warn" => tracing::Level::WARN,
            "error" => tracing::Level::ERROR,
            _ => tracing::Level::INFO,
        }
    }

    // 获取输出目标，如果未指定则返回默认值 "console"
    pub fn output(&self) -> &str {
        match &self.output {
            Some(output) => output.as_str(),
            None => "console", // 默认输出到控制台
        }
    }

    // 获取sqlx日志级别
    pub fn sqlx_level(&self) -> &str {
        match &self.sqlx_level {
            Some(level) => level.as_str(),
            None => "info", // 默认值
        }
    }

    // 获取特定组件的日志级别
    pub fn component_level(&self, component: &str) -> Option<&str> {
        match &self.components {
            Some(components) => components.get(component).map(|s| s.as_str()),
            None => None,
        }
    }
}