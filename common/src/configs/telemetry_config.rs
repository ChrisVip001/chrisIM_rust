use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct TelemetryConfig {
    pub enabled: bool,               // 是否启用链路追踪
    pub endpoint: String,            // Jaeger/OTLP终端点
    pub sampling_ratio: f64,         // 采样率: 0.0-1.0
    pub propagation: String,         // 传播方式: tracecontext, b3, jaeger
}
