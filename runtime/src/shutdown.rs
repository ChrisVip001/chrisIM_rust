//! 统一的优雅停机信号监听
//!
//! 四个服务共用同一套信号处理：SIGTERM（容器编排的标准停机信号）
//! 与 SIGINT（Ctrl+C）任一到达即返回，各服务在此基础上执行
//! 「摘除注册流量 → 停止接收新请求 → 排空存量请求 → 退出」的收尾流程。
//!
//! 此前各服务的信号处理各自实现且不一致：relation-service 完整，
//! api-gateway 发了信号却没人消费，msg-server / msg-gateway 完全没有。

use tracing::info;

/// 等待停机信号（SIGTERM 或 SIGINT），返回即表示应开始优雅退出
pub async fn wait_for_shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("无法安装Ctrl+C处理器");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("无法安装SIGTERM处理器")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("收到停机信号，开始优雅退出");
}
