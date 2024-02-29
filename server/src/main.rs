mod agent;
mod app;
mod client;

use clap::Parser;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(
        short,
        long,
        value_parser,
        default_value = "0.0.0.0",
        help = "指定要监听的主机ip"
    )]
    server_ip: String,
    #[clap(
        short,
        long,
        value_parser,
        default_value_t = 80,
        help = "提供给外网用户访问的端口"
    )]
    user_port: u16,
    #[clap(
        short,
        long,
        value_parser,
        default_value_t = 9527,
        help = "提供给客户端访问的端口"
    )]
    client_port: u16,

    #[clap(
        short,
        long,
        value_parser,
        default_value = "info",
        help = "指定日志级别"
    )]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    println!("args:{:?}", args);

    std::env::set_var("RUST_LOG", args.log_level);
    tracing_subscriber::fmt::init();

    let app = app::App::new().await?;

    app.run(&args.server_ip, args.user_port, args.client_port)
        .await?;

    Ok(())
}
