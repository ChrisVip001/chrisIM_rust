use tracing::Level;

use common::config::AppConfig;

use msg_server::productor::ChatRpcService;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();
    let config = AppConfig::from_file(Some("./config/config.yaml")).unwrap();
    ChatRpcService::start(&config).await;
}
