use anyhow::Result;
use tokio::sync::oneshot;
use tracing::{error, info, warn};

use crate::config::AppConfig;
use crate::Error;
use crate::service_register_center::service_register_center;

/// 添加rustls初始化函数
/// 在应用启动前调用此函数，以初始化rustls CryptoProvider
pub fn init_rustls() {
    // 初始化并安装默认的CryptoProvider
    // 忽略错误，这样多线程环境下只有第一次调用会成功，其他调用会静默失败
    let _ = rustls::crypto::ring::default_provider().install_default();
    info!("已安装rustls默认CryptoProvider");
}

/// 处理优雅关闭
///
/// 监听关闭信号并在收到信号后从服务注册中心注销服务
///
/// # 参数
/// * `tx` - 关闭通知发送端
/// * `service_id` - 要注销的服务ID
/// * `config` - 应用配置
///
/// # 返回
/// 成功返回 Ok(()), 失败返回 Error
pub async fn shutdown_signal(
    tx: oneshot::Sender<()>,
    service_id: String,
    config: &AppConfig,
) -> Result<(), Error> {
    use tokio::signal;

    // 获取服务注册中心
    let service_registry = service_register_center(config);

    // 监听 Ctrl+C 信号
    let ctrl_c = async {
        signal::ctrl_c().await.expect("无法安装Ctrl+C处理器");
    };

    // 在Unix系统上监听 SIGTERM 信号
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("无法安装SIGTERM处理器")
            .recv()
            .await;
    };

    // 在非Unix系统上创建一个永不返回的future
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    // 等待任一信号
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("接收到关闭信号，准备优雅关闭...");

    // 从服务注册中心注销服务
    match service_registry.deregister(&service_id).await {
        Ok(_) => info!("已从服务注册中心注销服务: {}", service_id),
        Err(e) => error!("从服务注册中心注销服务失败: {}", e),
    }

    // 发送关闭信号通知服务器关闭
    if let Err(_) = tx.send(()) {
        warn!("无法发送关闭信号，接收端可能已关闭");
    }

    info!("服务关闭准备完成");
    Ok(())
}
