use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use clap::Parser;
use proto::{Pack, Packet, UnPack};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{mpsc::Sender, Mutex},
    task::JoinHandle,
};
use tracing::{debug, error, info, trace, warn};

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

    let stream = TcpStream::connect(format!("{}:{}", args.server_ip, args.server_port))
        .await
        .unwrap();

    let users_map = Arc::new(Mutex::new(HashMap::<
        String,
        (Sender<Packet>, JoinHandle<()>),
    >::new()));

    // 创建一个多发送者的channel
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Packet>(1000);
    let (mut stream_read, mut stream_write) = stream.into_split();

    // 发送bind
    let packet = Packet::new(
        proto::PackType::Bind,
        Some(proto::Bind {
            host: args.host.clone(),
        }),
    ).unwrap();
    let encoded = packet.pack().unwrap();
    debug!("发送bind数据包:{:x?}", encoded);
    if let Err(e) = stream_write.write(&encoded).await {
        error!("发送bind数据包失败; err = {:?}", e);
        return;
    }

    // 创建一个线程，通过rx接受数据包，收到就通过stream_write发送
    tokio::spawn(async move {
        while let Some(packet) = rx.recv().await {
            let encoded = packet.pack().unwrap();
            debug!("打包后的数据：{:x?}", encoded);
            if let Err(e) = stream_write.write(&encoded).await {
                error!("把客户端数据转发给用户出错; err = {:?}", e);
                return;
            }
            debug!("成功把客户端的数据转发给用户");
        }
    });



    loop {
        // 循环读取服务端发来的数据包，解析并处理
        debug!("等待服务端发来数据");
        let mut readed_len = 0;
        let header_size = proto::Header::size().unwrap();
        let mut buf = vec![0; header_size];

        while readed_len < header_size {
            let mut tmp_buf = vec![0; header_size - readed_len];
            let n = match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                stream_read.read(&mut tmp_buf),
            )
            .await
            {
                Ok(Ok(0)) => {
                    debug!("Connection closed");
                    break;
                }
                Ok(Ok(n)) => n,
                Ok(Err(e)) => {
                    error!("Failed to read from socket: {:?}", e);
                    break;
                }
                Err(_) => {
                    // 超时处理逻辑
                    info!("Read timeout, closing connection");
                    break;
                }
            };
            debug!("读取到:{}", n);
            buf = [&buf[..readed_len as usize], &tmp_buf[..n]].concat();
            readed_len += n;
            debug!(
                "读取:readed_len:{}, header_size:{}",
                readed_len, header_size
            );
        }
        if readed_len < header_size {
            continue;
        }
        debug!("读取完毕:{:x?}", &buf);
        let header = match proto::Header::unpack(&buf[..readed_len as usize]) {
            Ok(header) => header,
            Err(e) => {
                error!("failed to unpack header; err = {:?}", e);
                continue;
            }
        };

        if !header.check_flag() {
            error!("数据头校验失败");
            continue;
        }

        match header.pack_type {
            proto::PackType::Data => {
                let mut readed_len = 0;
                let mut buf = vec![0; header.body_size as usize];
                while readed_len < header.body_size {
                    let mut tmp_buf = vec![0; header.body_size as usize - readed_len as usize];
                    let n = match stream_read.read(&mut tmp_buf).await {
                        Ok(n) if n == 0 => {
                            debug!("服务端连接断开");
                            break;
                        }
                        Ok(n) => n,
                        Err(e) => {
                            error!("failed to read from socket; err = {:?}", e);
                            continue;
                        }
                    };
                    buf = [&buf[..readed_len as usize], &tmp_buf[..n]].concat();
                    readed_len += n as u64;
                }

                let data = match proto::Data::unpack(&buf[..readed_len as usize]) {
                    Ok(decoded) => decoded,
                    Err(e) => {
                        error!("failed to unpack data; err = {:?}", e);
                        continue;
                    }
                };

                trace!("读取到用户数据：{:?}", String::from_utf8_lossy(&data.data));
                debug!(
                    "共从服务端读取到 {} 字节数据",
                    header_size + readed_len as usize
                );
                let user_id = data.agent_ip_port.clone();
                let is_contain_user_id = { users_map.clone().lock().await.contains_key(&user_id) };
                debug!("用户：{}, 是否存在：{}", user_id, is_contain_user_id);
                // 如果是新的连接，就建立一个线程把数据转发给本地服务，并读取本地数据返回给中转服务器
                let users_map_clone = users_map.clone();
                if !is_contain_user_id {
                    let (client_tx, mut client_rx) = tokio::sync::mpsc::channel::<Packet>(1000);
                    // 连接本地服务
                    let stream =
                        match TcpStream::connect(format!("{}:{}", args.ip, args.port)).await {
                            Ok(stream) => {
                                debug!("客户端连接成功");
                                stream
                            }
                            Err(e) => {
                                error!("连接失败; err = {:?}", e);
                                continue;
                            }
                        };

                    let tx_clone = tx.clone();
                    let users_map_clone1 = users_map_clone.clone();
                    let users_map_clone2 = users_map_clone.clone();
                    let handle = tokio::spawn(async move {
                        let (mut stream_read, mut stream_write) = stream.into_split();

                        let h = tokio::spawn(async move {
                            // 接收客户端来的数据，转发给本地服务
                            while let Some(packet) = client_rx.recv().await {
                                match packet.header.pack_type {
                                    proto::PackType::Data => {
                                        let decoded = match proto::Data::unpack(&packet.data) {
                                            Ok(decoded) => decoded,
                                            Err(e) => {
                                                error!("failed to unpack data; err = {:?}", e);
                                                return;
                                            }
                                        };
                                        if let Err(e) = stream_write.write(&decoded.data).await {
                                            error!("把客户端数据转发给用户出错; err = {:?}", e);
                                            return;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        });

                        // 读取本地服务数据，发给中转服务器
                        loop {
                            let mut buf = vec![0; 1024 * 1024];
                            let n = match stream_read.read(&mut buf).await {
                                Ok(n) if n == 0 => {
                                    debug!("客户端连接断开");
                                    // 本地服务断开，就移除
                                    users_map_clone1.lock().await.remove(&user_id);
                                    break;
                                }
                                Ok(n) => n,
                                Err(e) => {
                                    error!("从客户端读取数据出错; err = {:?}", e);
                                    // 本地服务断开，就移除
                                    users_map_clone1.lock().await.remove(&user_id);
                                    break;
                                }
                            };
                            trace!(
                                "从客户端读取数据完毕，转发给用户:{:?}",
                                String::from_utf8_lossy(&buf[..n])
                            );

                            let packet = proto::Packet::new(
                                proto::PackType::Data,
                                Some(proto::Data {
                                    agent_ip_port: user_id.to_string(),
                                    host: None,
                                    data: buf[..n].to_vec(),
                                }),
                            )
                            .unwrap();

                            if let Err(e) = tx_clone.send(packet).await {
                                error!("发送数据给用户出错; err = {:?}", e);
                                break;
                            }
                        }

                        h.await.unwrap();
                    });

                    {
                        users_map_clone2
                            .lock()
                            .await
                            .insert(data.agent_ip_port.clone(), (client_tx, handle));
                    }
                }

                let client_tx = {
                    users_map
                        .clone()
                        .lock()
                        .await
                        .get(&data.agent_ip_port)
                        .unwrap()
                        .0
                        .clone()
                };
                debug!("发送数据给客户端channel");
                if let Err(e) = client_tx
                    .send(Packet::new(proto::PackType::Data, Some(data)).unwrap())
                    .await
                {
                    error!("发送数据给用户出错; err = {:?}", e);
                    continue;
                }
            }
            proto::PackType::DataEnd => {
                let mut readed_len = 0;
                let mut buf = vec![0; header.body_size as usize];
                if header.body_size > 0 {
                    while readed_len < header.body_size {
                        let mut tmp_buf = vec![0; header.body_size as usize - readed_len as usize];
                        let n = match stream_read.read(&mut tmp_buf).await {
                            Ok(n) if n == 0 => {
                                debug!("服务端连接断开");
                                break;
                            }
                            Ok(n) => n,
                            Err(e) => {
                                error!("failed to read from socket; err = {:?}", e);
                                continue;
                            }
                        };
                        buf = [&buf[..readed_len as usize], &tmp_buf[..n]].concat();
                        readed_len += n as u64;
                    }
                }
                debug!("数据读取完毕");
            }
            _ => {
                error!("未知的数据包类型");
                break;
            }
        }
    }
}
