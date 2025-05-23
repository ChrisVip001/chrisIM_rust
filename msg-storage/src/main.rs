use tracing::Level;

use common::config::ConfigLoader;
use msg_storage::clean_receive_box;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_line_number(true)
        .init();

    // 初始化全局配置
    ConfigLoader::init_global().expect("初始化全局配置失败");

    // 确保全局配置可以正常访问
    let config = ConfigLoader::get_global().expect("获取全局配置失败");

    // start cleaner
    clean_receive_box(&config).await;

    // start rpc service
    // DbRpcService::start(&config).await;
}
