use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct LogConfig {
    pub level: String,
    pub output: String,
    pub sqlx_level: Option<String>,    // SQL查询日志级别
    pub components: Option<std::collections::HashMap<String, String>>, // 其他组件的日志级别
    pub format: Option<String>,        // 日志输出格式: plain或json
    pub rolling: Option<LogRollingConfig>, // 日志滚动配置
}

#[derive(Debug, Deserialize, Clone)]
pub struct LogRollingConfig {
    pub rotation: String,              // 滚动策略: hourly, daily, never
    pub max_files: Option<usize>,      // 最大保留文件数量
    pub directory: Option<String>,     // 日志文件目录
    pub filename_prefix: Option<String>, // 日志文件名前缀
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

    // 获取日志滚动配置
    pub fn rolling(&self) -> Option<&LogRollingConfig> {
        self.rolling.as_ref()
    }
}

impl LogRollingConfig {
    pub fn rotation_type(&self) -> &str {
        &self.rotation
    }

    pub fn max_files(&self) -> usize {
        self.max_files.unwrap_or(7) // 默认保留7个文件
    }

    pub fn directory(&self) -> &str {
        self.directory.as_deref().unwrap_or("logs") // 默认logs目录
    }

    pub fn filename_prefix(&self) -> &str {
        self.filename_prefix.as_deref().unwrap_or("") // 不使用默认前缀
    }


}