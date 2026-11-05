//! Prometheus 指标导出
//!
//! api-gateway 有 HTTP 路由，自己在 `metrics` 模块里安装 recorder 并暴露 `/metrics`；
//! `msg-server` / `msg-gateway` 没有 HTTP 业务端点，用这里的监听器单独起一个端口。
//!
//! 约束：
//! - 一个进程只能安装一个全局 recorder，重复调用直接返回，不会 panic；
//! - 初始化失败（端口被占等）只记 warn，不影响主服务启动 —— 指标不该成为单点。

use metrics_exporter_prometheus::PrometheusHandle;
use once_cell::sync::OnceCell;
use tracing::{info, warn};

static HANDLE: OnceCell<Option<PrometheusHandle>> = OnceCell::new();

/// 在 `0.0.0.0:{port}` 上启动 `/metrics` 并安装全局 recorder。
///
/// 返回 `true` 表示 recorder 可用（本次安装或之前已安装）。
///
/// **必须**在 Tokio 运行时内调用（服务的 `#[tokio::main]` 入口即可）：
/// 这里用 [`metrics_exporter_prometheus::PrometheusBuilder::build`] 拿
/// `(recorder, exporter)`，把 exporter 挂到当前运行时上。不能用
/// `install_recorder()` —— 那个方法只装 recorder，不 spawn 监听，
/// `/metrics` 会变成一个永远连不上的端口。
pub fn init(port: u16) -> bool {
    let installed = HANDLE.get_or_init(|| {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            // 单测等无运行时场景：没有监听端口的需求，直接跳过
            warn!("Prometheus 指标未启动：当前不在 Tokio 运行时内");
            return None;
        };

        let (recorder, exporter) = match metrics_exporter_prometheus::PrometheusBuilder::new()
            .with_http_listener(([0, 0, 0, 0], port))
            .build()
        {
            Ok(pair) => pair,
            Err(e) => {
                warn!("Prometheus 指标启动失败（端口 {}）: {}", port, e);
                return None;
            }
        };

        let handle = recorder.handle();
        if let Err(e) = metrics::set_global_recorder(recorder) {
            warn!("Prometheus recorder 安装失败: {}", e);
            return None;
        }

        // exporter 自带 upkeep 任务（直方图桶清理），挂到服务运行时上
        runtime.spawn(exporter);
        info!("Prometheus 指标已启动: 0.0.0.0:{}/metrics", port);
        Some(handle)
    });
    installed.is_some()
}

/// 渲染当前指标文本（给已有 HTTP 服务的 `/metrics` 路由复用）
pub fn render() -> String {
    HANDLE
        .get()
        .and_then(|h| h.as_ref())
        .map(|h| h.render())
        .unwrap_or_default()
}

/// 指标是否已启用（用于埋点处判断是否需要付出取指成本）
pub fn enabled() -> bool {
    matches!(HANDLE.get(), Some(Some(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 起一个临时端口，抓一次 `/metrics`，确认「装 recorder」和「起监听」都真的发生了
    ///
    /// 这是当初 `install_recorder()` 只装 recorder 不起服务的回归用例：
    /// 那时端口永远连不上，埋点全成了死数据。
    #[tokio::test]
    async fn serves_metrics_on_configured_port() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };

        assert!(init(port), "指标监听应当在 Tokio 运行时内启动成功");
        assert!(enabled());
        metrics::counter!("runtime_test_total").increment(1);

        let mut body = String::new();
        // 监听就绪是异步的，重试几次再断言
        for _ in 0..20 {
            if let Ok(resp) = reqwest::get(format!("http://127.0.0.1:{port}/metrics")).await {
                body = resp.text().await.unwrap_or_default();
                if body.contains("runtime_test_total") {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        assert!(
            body.contains("runtime_test_total"),
            "抓取到的指标里应包含刚打点的计数器，实际内容: {body}"
        );
        assert!(render().contains("runtime_test_total"));
    }
}
