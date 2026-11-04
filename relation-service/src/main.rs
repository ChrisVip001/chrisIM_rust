use anyhow::Result;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use common::config::{AppConfig, Component, ConfigLoader};
use common::grpc::LoggingInterceptor;
use common::grpc_client::base::{register_service, service_register_center};
use common::proto::friend::friend_service_server::FriendServiceServer;
use common::proto::group::group_service_server::GroupServiceServer;
use common::proto::user::user_service_server::UserServiceServer;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::watch;
use tonic::transport::Server;
use tonic_health::server::health_reporter;
use tonic_reflection::server::Builder as ReflectionBuilder;
use tracing::{error, info};

mod friend;
mod group;
mod user;

use friend::service::friend_service::FriendServiceImpl;
use group::service::group_service::GroupServiceImpl;
use user::service::user_service::UserServiceImpl;

// 三个服务的 proto 文件描述符，用于 gRPC 反射
const USER_FD_SET: &[u8] = common::proto::user::FILE_DESCRIPTOR_SET;
const FRIEND_FD_SET: &[u8] = common::proto::friend::FILE_DESCRIPTOR_SET;
const GROUP_FD_SET: &[u8] = common::proto::group::FILE_DESCRIPTOR_SET;

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化 rustls 加密提供程序
    common::service::init_rustls();

    // 从环境变量获取配置文件路径
    let config_path = env::var("CONFIG_PATH").unwrap_or_else(|_| "./config/config.yaml".to_string());
    info!("使用配置文件: {}", config_path);

    // 初始化全局配置
    let app_config = AppConfig::from_file(Some(&config_path))
        .expect(&format!("无法从路径加载配置: {}", config_path));
    ConfigLoader::set_global(app_config);
    let config = ConfigLoader::get_global().expect("获取全局配置失败");

    // 初始化日志和链路追踪
    if config.telemetry.enabled {
        common::logging::init_telemetry(&config, "relation-service")?;
        info!("链路追踪功能已启用: {}", config.telemetry.endpoint);
    } else {
        common::logging::init_from_config(&config, "relation-service")?;
        info!("链路追踪功能未启用，仅初始化日志系统");
    }

    info!("正在启动关系服务（用户/好友/群组）...");

    // 初始化数据库连接池
    let db_pool = match PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database.pg_url())
        .await
    {
        Ok(pool) => {
            info!("数据库连接成功");
            pool
        }
        Err(err) => {
            error!("数据库连接失败: {}", err);
            return Err(err.into());
        }
    };

    // 先注册三个服务到注册中心（friend/group 初始化时会通过服务发现解析 user 服务，
    // 合并后由本进程自身提供，因此必须先完成注册）
    let user_service_id = register_service(&config, Component::UserServer).await?;
    let friend_service_id = register_service(&config, Component::FriendServer).await?;
    let group_service_id = register_service(&config, Component::GroupServer).await?;
    let service_ids = vec![user_service_id, friend_service_id, group_service_id];
    info!("已注册 user/friend/group 三个服务到注册中心");

    // 初始化三个业务服务实例（可 Clone，供多个监听端口复用）
    // 依赖顺序：friend/group 直调 user；user 直调 friend（构造后绑定，解决循环依赖）
    let user_service = UserServiceImpl::new(db_pool.clone()).await?;
    let user_arc = Arc::new(user_service);
    let friend_service = FriendServiceImpl::new(db_pool.clone(), user_arc.clone()).await?;
    let group_service = GroupServiceImpl::new(db_pool.clone(), user_arc.clone()).await?;
    let friend_arc = Arc::new(friend_service);
    user_arc
        .bind_friend_service(friend_arc.clone())
        .map_err(|_| anyhow::anyhow!("绑定好友服务失败（重复绑定）"))?;
    info!("用户/好友/群组服务初始化完成（user↔friend/group 进程内直调）");

    // 沿用原先 user/friend/group 各自的监听地址，保持部署配置兼容
    let mut addrs: Vec<SocketAddr> = Vec::new();
    for rpc_cfg in [&config.rpc.user, &config.rpc.friend, &config.rpc.group] {
        let addr = format!("{}:{}", rpc_cfg.host, rpc_cfg.port).parse::<SocketAddr>()?;
        addrs.push(addr);
    }

    // 优雅关闭通道：信号到达后注销三个服务并通知所有监听器退出
    // 使用 watch 而非 oneshot：三个监听器都需要收到关闭通知
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let config_clone = config.clone();
    let shutdown_task = tokio::spawn(async move {
        use tokio::signal;

        let ctrl_c = async { signal::ctrl_c().await.expect("无法安装Ctrl+C处理器") };
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

        info!("接收到关闭信号，准备优雅关闭...");
        let service_registry = service_register_center(&config_clone);
        for id in &service_ids {
            match service_registry.deregister(id).await {
                Ok(_) => info!("已从服务注册中心注销服务: {}", id),
                Err(e) => error!("从服务注册中心注销服务失败: {}", e),
            }
        }
        let _ = shutdown_tx.send(true);
        info!("服务关闭准备完成");
    });

    // 为每个监听地址构建服务器：同一端口同时提供 user/friend/group 三组 gRPC 服务
    let mut servers = Vec::new();
    for addr in addrs {
        let user_svc = user_arc.as_ref().clone();
        let friend_svc = friend_arc.as_ref().clone();
        let group_svc = group_service.clone();
        let mut shutdown_rx = shutdown_rx.clone();

        // 反射服务：合并注册三个 proto 的文件描述符
        let reflection_service = ReflectionBuilder::configure()
            .register_encoded_file_descriptor_set(USER_FD_SET)
            .register_encoded_file_descriptor_set(FRIEND_FD_SET)
            .register_encoded_file_descriptor_set(GROUP_FD_SET)
            .build_v1()?;

        // gRPC 健康检查
        let (mut health_reporter, health_service) = health_reporter();
        health_reporter
            .set_serving::<UserServiceServer<UserServiceImpl>>()
            .await;
        health_reporter
            .set_serving::<FriendServiceServer<FriendServiceImpl>>()
            .await;
        health_reporter
            .set_serving::<GroupServiceServer<GroupServiceImpl>>()
            .await;

        let server = Server::builder()
            .add_service(health_service)
            .add_service(UserServiceServer::with_interceptor(
                user_svc,
                LoggingInterceptor::new(),
            ))
            .add_service(FriendServiceServer::with_interceptor(
                friend_svc,
                LoggingInterceptor::new(),
            ))
            .add_service(GroupServiceServer::with_interceptor(
                group_svc,
                LoggingInterceptor::new(),
            ))
            .add_service(reflection_service)
            .serve_with_shutdown(addr, async move {
                let _ = shutdown_rx.changed().await;
                info!("接收到关闭信号，gRPC服务器准备关闭");
            });

        servers.push(server);
        info!("gRPC 服务就绪（user/friend/group），监听地址: {}", addr);
    }

    // 并发运行三个监听器
    let (r1, r2, r3) = tokio::join!(servers.remove(0), servers.remove(0), servers.remove(0));
    for (idx, r) in [r1, r2, r3].into_iter().enumerate() {
        if let Err(e) = r {
            error!("gRPC服务器[{}]错误: {}", idx, e);
        }
    }

    let _ = shutdown_task.await?;

    // 关闭链路追踪，确保数据都被发送
    if config.telemetry.enabled {
        info!("正在关闭链路追踪...");
        common::logging::shutdown_telemetry();
    }

    info!("关系服务已完全关闭");
    Ok(())
}
