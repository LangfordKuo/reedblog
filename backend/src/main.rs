//! reedblog 后端入口。配置文件路径默认 backend/config.toml，可用环境变量 REEDBLOG_CONFIG 覆盖。

#[tokio::main]
async fn main() {
    let config_path =
        std::env::var("REEDBLOG_CONFIG").unwrap_or_else(|_| "config.toml".to_string());
    if let Err(e) = reedblog_backend::run(&config_path).await {
        eprintln!("[reedblog] 启动失败: {e}");
        std::process::exit(1);
    }
}
