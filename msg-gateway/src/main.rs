use tracing::Level;

use common::config::AppConfig;
use msg_gateway::ws_server::WsServer;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();
    WsServer::start(AppConfig::from_file(Some("./config/config.yaml")).unwrap()).await
}
#[cfg(test)]
mod tests {
    use common::message::msg_service_server::MsgServiceServer;
    use common::message::Msg;
    use msg_gateway::rpc;
    use tonic::server::NamedService;

    #[test]
    fn test_load() {
        let msg = Msg::default();
        println!("{}", serde_json::to_string(&msg).unwrap());
        println!(
            "{:?}",
            <MsgServiceServer<rpc::MsgRpcService> as NamedService>::NAME
        );
    }
}
