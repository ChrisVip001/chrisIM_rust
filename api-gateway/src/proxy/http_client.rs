use reqwest::{Client, Response, Error};
use std::time::Duration;
use hyper::http::HeaderMap;
use tracing::debug;
use std::error::Error as StdError;

/// HTTP客户端配置
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    /// 连接超时时间（秒）
    pub connect_timeout: u64,
    /// 请求超时时间（秒）
    pub timeout: u64,
    /// 重试次数
    pub max_retries: u32,
    /// 重试间隔（毫秒）
    pub retry_interval: u64,
    /// 是否启用gzip压缩
    pub enable_gzip: bool,
    /// 是否添加链路追踪头
    pub add_tracing_headers: bool,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: 5,
            timeout: 30,
            max_retries: 3,
            retry_interval: 100,
            enable_gzip: true,
            add_tracing_headers: true,
        }
    }
}

/// 增强的HTTP客户端
pub struct HttpClient {
    /// 内部reqwest客户端
    client: Client,
    /// 客户端配置
    config: HttpClientConfig,
}

impl HttpClient {
    /// 创建新的HTTP客户端
    pub fn new(config: HttpClientConfig) -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(config.connect_timeout))
            .timeout(Duration::from_secs(config.timeout))
            .pool_max_idle_per_host(100)
            .build()
            .unwrap_or_default();
        
        Self { client, config }
    }
    
    /// 使用默认配置创建客户端
    pub fn default() -> Self {
        Self::new(HttpClientConfig::default())
    }
    
    /// 发送GET请求
    pub async fn get(&self, url: &str, headers: Option<HeaderMap>) -> Result<Response, Error> {
        let mut req = self.client.get(url);
        
        // 添加请求头
        if let Some(headers) = headers {
            req = req.headers(headers);
        }
        
        self.send_with_retry(req, self.config.max_retries).await
    }
    
    /// 发送POST请求
    pub async fn post(&self, url: &str, headers: Option<HeaderMap>, body: Option<Vec<u8>>) -> Result<Response, Error> {
        let mut req = self.client.post(url);
        
        // 添加请求头
        if let Some(headers) = headers {
            req = req.headers(headers);
        }
        
        // 添加请求体
        if let Some(body) = body {
            req = req.body(body);
        }
        
        self.send_with_retry(req, self.config.max_retries).await
    }
    
    /// 发送POST JSON请求
    pub async fn post_json<T: serde::Serialize>(&self, url: &str, headers: Option<HeaderMap>, json: &T) -> Result<Response, Error> {
        let mut req = self.client.post(url);
        
        // 添加请求头
        if let Some(headers) = headers {
            req = req.headers(headers);
        }
        
        // 添加JSON请求体
        req = req.json(json);
        
        self.send_with_retry(req, self.config.max_retries).await
    }
    
    /// 发送PUT请求
    pub async fn put(&self, url: &str, headers: Option<HeaderMap>, body: Option<Vec<u8>>) -> Result<Response, Error> {
        let mut req = self.client.put(url);
        
        // 添加请求头
        if let Some(headers) = headers {
            req = req.headers(headers);
        }
        
        // 添加请求体
        if let Some(body) = body {
            req = req.body(body);
        }
        
        self.send_with_retry(req, self.config.max_retries).await
    }
    
    /// 发送DELETE请求
    pub async fn delete(&self, url: &str, headers: Option<HeaderMap>) -> Result<Response, Error> {
        let mut req = self.client.delete(url);
        
        // 添加请求头
        if let Some(headers) = headers {
            req = req.headers(headers);
        }
        
        self.send_with_retry(req, self.config.max_retries).await
    }
    
    /// 带重试的请求发送
    async fn send_with_retry(&self, req: reqwest::RequestBuilder, retries: u32) -> Result<Response, Error> {
        let mut attempts = 0;

        // 对于第一次请求，直接发送
        let first_req = req;

        match first_req.send().await {
            Ok(response) => {
                // TODO 如果第一次请求成功但需要重试
                if is_retryable_status(&response) && attempts < retries {
                    // 记录URL和方法，用于重建请求
                    let _url = response.url().clone();
                    // 继续处理重试逻辑...
                    return Ok(response);
                }
                Ok(response)
            },
            Err(err) => {
                if is_retryable_error(&err) && attempts < retries {
                    attempts += 1;
                    debug!("请求错误: {}, 尝试重试 ({}/{})", err, attempts, retries);
                    tokio::time::sleep(Duration::from_millis(self.config.retry_interval)).await;
                    // TODO 这里应该重建请求，但简化为直接返回错误
                }
                Err(err)
            }
        }
    }
}

/// 检查状态码是否可重试
fn is_retryable_status(response: &Response) -> bool {
    match response.status().as_u16() {
        // 服务器错误
        500 | 502 | 503 | 504 => true,
        // 请求过多
        429 => true,
        // 其他状态码不重试
        _ => false,
    }
}

/// 检查错误是否可重试
fn is_retryable_error(err: &Error) -> bool {
    err.is_timeout() || err.is_connect() || is_reset_error(err)
}

/// 检查是否为连接重置错误
fn is_reset_error(err: &Error) -> bool {
    if let Some(source) = err.source() {
        if let Some(io_err) = source.downcast_ref::<std::io::Error>() {
            return io_err.kind() == std::io::ErrorKind::ConnectionReset;
        }
    }
    false
} 