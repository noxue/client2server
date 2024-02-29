mod app;
mod client;
mod service;

use anyhow::bail;
use app::App;
use clap::Parser;
use proto::{Pack, PackType, Packet, UnPack};
use std::{collections::HashMap, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{
        mpsc::{Receiver, Sender},
        Mutex,
    },
};
use tracing::{debug, error};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short = 'I', long, value_parser, help = "中转服务器IP")]
    server_ip: String,
    #[clap(
        short = 'o',
        long,
        value_parser,
        default_value_t = 9527,
        help = "中转服务器端口"
    )]
    server_port: u16,

    #[clap(
        short,
        long,
        value_parser,
        default_value = "127.0.0.1",
        help = "提供服务的ip"
    )]
    ip: String,
    #[clap(
        short,
        long,
        value_parser,
        help = "提供服务的端口, 也就是你的程序监听的端口"
    )]
    port: u16,

    #[clap(short = 'd', long = "domain", value_parser, help = "指定使用的域名")]
    host: String,

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
async fn main() {
    let args = Args::parse();

    std::env::set_var("RUST_LOG", args.log_level);
    tracing_subscriber::fmt::init();

    let mut app = App::new().await.unwrap();
    app.run(
        args.server_ip,
        args.server_port,
        args.ip,
        args.port,
        args.host,
    )
    .await
    .unwrap();
}
