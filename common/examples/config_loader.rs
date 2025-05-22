use common::config::{AppConfig, Component, ConfigLoader};
use std::sync::Arc;
use tracing::{info, Level};
use tracing_subscriber::{filter::LevelFilter, FmtSubscriber};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("设置日志订阅器失败");

    // 示例1：初始化全局配置单例，以便在任何地方直接访问
    info!("初始化全局配置单例");
    ConfigLoader::init_global()?;
    
    if let Some(global_config) = ConfigLoader::get_global() {
        info!("全局配置数据库用户: {}", global_config.database.postgres.user);
        info!("全局配置Redis地址: {}", global_config.redis.url());
    }

    // 示例2：加载用户服务配置（合并全局配置和服务特定配置）
    info!("\n加载用户服务配置");
    let mut user_loader = ConfigLoader::new(Component::UserServer);
    let user_config = user_loader.load()?;
    info!(
        "用户服务配置数据库用户: {}",
        user_config.database.postgres.user
    );
    info!("用户服务配置端口: {}", user_config.server.port);
    
    // 示例3：加载好友服务配置（合并全局配置和服务特定配置）
    info!("\n加载好友服务配置");
    let mut friend_loader = ConfigLoader::new(Component::FriendServer);
    let friend_config = friend_loader.load()?;
    info!(
        "好友服务配置数据库用户: {}",
        friend_config.database.postgres.user
    );
    info!("好友服务配置端口: {}", friend_config.server.port);
    
    // 示例4：直接使用全局配置（不合并服务特定配置）
    info!("\n直接使用全局配置");
    let global_config = AppConfig::from_file(None)?;
    info!("全局配置Redis端口: {}", global_config.redis.port);

    // 示例5：演示如何在服务中使用配置
    info!("\n服务使用配置示例");
    start_service(Component::UserServer)?;

    Ok(())
}

/// 模拟一个服务的启动过程
fn start_service(component: Component) -> Result<(), Box<dyn std::error::Error>> {
    // 1. 初始化服务配置
    let mut loader = ConfigLoader::new(component.clone());
    let config = loader.load()?;
    
    // 2. 将配置设为全局单例，以便在任何地方访问
    ConfigLoader::set_global((*config).clone());
    
    // 3. 使用配置启动服务
    let server_url = config.server.server_url();
    info!("服务 {:?} 启动在 {}", component, server_url);
    
    // 4. 设置数据库连接
    info!(
        "连接到数据库: {}@{}:{}/{}",
        config.database.postgres.user,
        config.database.postgres.host,
        config.database.postgres.port,
        config.database.postgres.database
    );
    
    // 5. 设置Redis连接
    info!("连接到Redis: {}", config.redis.url());
    
    // 6. 可选：启动配置文件变更监控
    #[cfg(feature = "dynamic-config")]
    {
        info!("启动配置文件变更监控");
        if let Err(e) = ConfigLoader::watch_config_changes(component) {
            eprintln!("启动配置文件监控失败: {}", e);
        }
    }
    
    Ok(())
} 