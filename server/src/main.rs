mod app;
mod agent;
mod client;

use agent::Agent;
use anyhow::bail;
use clap::Parser;
use client::Client;
use proto::{Pack, PackType, Packet, UnPack};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;
use tokio::time;
use tracing::{debug, error, info, trace};

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

    info!(
        "服务端启动成功，监听地址: {}，用户端口：{},  客户端端口：{}",
        args.server_ip, args.user_port, args.client_port
    );

    app.run(&args.server_ip, args.user_port, args.client_port).await?;

    Ok(())
}
