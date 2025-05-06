mod api;
mod config;
mod maxmind;
mod scheduler;

use api::{create_router, IpApiHandler};
use maxmind::{MaxmindReader, MaxmindUpdater};
use scheduler::Scheduler;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // 加载配置
    let config = config::init().map_err(|e| format!("配置初始化失败: {}", e))?;
    tracing::info!("配置加载成功");
    
    // 创建MaxMind数据库更新器
    let maxmind_config = Arc::new(config.maxmind.clone());
    let mut updater = MaxmindUpdater::new(maxmind_config.clone());
    
    // 创建MaxMind数据库读取器
    let reader = MaxmindReader::new(maxmind_config.clone());
    let reader_arc = Arc::new(RwLock::new(reader));
    
    // 执行初始数据库更新
    tracing::info!("初始化MaxMind数据库...");
    updater.update().await.map_err(|e| format!("MaxMind数据库初始化失败: {}", e))?;
    
    // 加载数据库
    {
        let mut reader = reader_arc.write().await;
        reader.load_databases().map_err(|e| format!("加载MaxMind数据库失败: {}", e))?;
    }

    // 设置更新定时任务
    let reader_arc_clone = reader_arc.clone();
    let mut scheduler = Scheduler::new();
    
    scheduler.schedule_daily("maxmind_db_update", 0, 0, move || {
        let updater_config = maxmind_config.clone();
        let reader_arc_update = reader_arc_clone.clone();
        
        tokio::spawn(async move {
            let mut updater = MaxmindUpdater::new(updater_config);
            
            if let Err(e) = updater.update().await {
                tracing::error!("MaxMind更新失败: {}", e);
                return;
            }
            
            let mut reader = reader_arc_update.write().await;
            if let Err(e) = reader.load_databases() {
                tracing::error!("重新加载MaxMind数据库失败: {}", e);
            }
        });
        
        Ok(())
    });
    
    // 启动定时任务调度器
    scheduler.start().await;
    
    // 创建HTTP路由
    let ip_handler = IpApiHandler::new(reader_arc.clone());
    let app = create_router(ip_handler);
    
    // 启动HTTP服务器
    let addr: SocketAddr = format!("0.0.0.0:{}", config.app.port)
        .parse()
        .expect("无效的地址格式");
    tracing::info!("IP API服务器启动, 监听地址: {}", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
        
    Ok(())
}
