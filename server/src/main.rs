use clap::Parser;
use proto::{Pack, PackType, UnPack};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
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

    let user_listener = TcpListener::bind(format!("{}:{}", args.server_ip, args.user_port)).await?;
    let client_listener =
        TcpListener::bind(format!("{}:{}", args.server_ip, args.client_port)).await?;

    info!(
        "服务端启动成功，监听地址: {}，用户端口：{},  客户端端口：{}",
        args.server_ip, args.user_port, args.client_port
    );

    let client_conn = Arc::new(Mutex::new(None));

    let client_conn_clone = client_conn.clone();
    tokio::spawn(async move {
        loop {
            let (socket, addr) = client_listener.accept().await.unwrap();
            {
                *client_conn_clone.lock().await = Some(socket);
            }
            info!("接收到客户端连接: {}", addr);
        }
    });

    loop {
        let (mut socket, addr) = user_listener.accept().await?;
        let ip_port = addr.to_string();
        debug!("Accepted connection from: {}", ip_port);

        let client_conn = client_conn.clone();
        // 使用Tokio的任务来异步处理每个连接
        tokio::spawn(async move {
            debug!("Handling connection from: {}", ip_port);
            // let socket = binding.get_mut(&ip_port).unwrap();
            // let (mut socket, mut socket) = socket.split();
            let mut buf = vec![0; 1024*1024];

            // 对127.0.0.1:8000 发起连接
            // let mut stream = TcpStream::connect("127.0.0.1:8000").await.unwrap();

            debug!("Reading from socket:{}", ip_port);
            // 在一个循环中读取数据，直到连接被关闭
            let n = match time::timeout(Duration::from_secs(3), socket.read(&mut buf)).await {
                Ok(Ok(0)) => {
                    debug!("Connection closed");
                    return;
                }
                Ok(Ok(n)) => n,
                Ok(Err(e)) => {
                    error!("Failed to read from socket: {:?}", e);
                    return;
                }
                Err(_) => {
                    // 超时处理逻辑
                    info!("Read timeout, closing connection");
                    return;
                }
            };

            let packet = proto::Packet::new(
                PackType::Data,
                Some(proto::Data {
                    user_id: ip_port.clone(),
                    data: buf[..n].to_vec(),
                }),
            )
            .unwrap();

            let encoded = packet.pack().unwrap();

            debug!("打包的数据:{:x?}", encoded);

            let binding = client_conn.clone();
            let mut binding = binding.lock().await;
            // let stream = binding.as_mut().unwrap();
            let stream = match binding.as_mut() {
                Some(stream) => stream,
                None => {
                    error!("未获取到客户端连接");
                    return;
                }
            };

            // 打印接收到的数据
            trace!("{}", String::from_utf8_lossy(&buf[..n]));
            if let Err(e) = stream.write(&encoded).await {
                error!("发送数据给客户端出错：{:?}", e);
                return;
            }

            // 马上刷新标准输出
            // std::io::stdout().flush().unwrap();

            // 检查获取到的数据 如果读取到 \r\n\r\n 表示请求头结束
            let header_end_pos =
                if let Some(index) = String::from_utf8_lossy(&buf[..n]).find("\r\n\r\n") {
                    index
                } else {
                    error!("Failed to find end of header");
                    return;
                };

            // 获取http头的Content-Length，然后读取指定长度的数据
            let content_length = if let Some(index) =
                String::from_utf8_lossy(&buf[..header_end_pos]).find("Content-Length: ")
            {
                let start = index + 16;
                let end = String::from_utf8_lossy(&buf[start..header_end_pos])
                    .find("\r\n")
                    .unwrap_or(header_end_pos - start);
                String::from_utf8_lossy(&buf[start..start + end])
                    .parse::<usize>()
                    .unwrap_or(0)
            } else {
                0
            };

            // body限制小于20M
            if content_length > 0 && content_length < 20 * 1024 * 1024 {
                // 第一个数据包可能已经包含了部分body内容，所以需要减去已经读取的部分
                let content_length = content_length - (n - (header_end_pos + 4));
                let mut body = vec![0; content_length];
                if let Err(e) = socket.read_exact(&mut body).await {
                    error!("Failed to read body: {:?}", e);
                    return;
                }
                trace!("{}", String::from_utf8_lossy(&body));

                let packet = proto::Packet::new(
                    PackType::Data,
                    Some(proto::Data {
                        user_id: ip_port.clone(),
                        data: body,
                    }),
                )
                .unwrap();

                let encoded = packet.pack().unwrap();

                debug!("打包的数据:{:x?}", encoded);
                stream.write(&encoded).await.unwrap();
            }

            let packet = proto::Packet::new(
                PackType::DataEnd,
                Some(proto::Data {
                    user_id: ip_port.clone(),
                    data: vec![],
                }),
            )
            .unwrap();
            let encoded = packet.pack().unwrap();
            debug!("打包的数据:{:x?}", encoded);
            stream.write(&encoded).await.unwrap();

            let header_size = proto::Header::size().unwrap();
            loop {
                let mut readed_len = 0;
                let mut buf = vec![0; header_size];
                while readed_len < header_size {
                    let mut tmp_buf = vec![0; header_size - readed_len];
                    let n = match stream.read(&mut tmp_buf).await {
                        Ok(n) if n == 0 => {
                            debug!("连接断开");
                            return;
                        }
                        Ok(n) => n,
                        Err(e) => {
                            error!("从客户端读取头数据出错; err = {:?}\nbuf:{:?}", e, &buf);
                            break;
                        }
                    };
                    buf = [&buf[..readed_len as usize], &tmp_buf[..n]].concat();
                    readed_len += n;
                }
                debug!("收到的打包数据:{:x?}", &buf[..header_size]);
                let header = match proto::Header::unpack(&buf[..header_size]) {
                    Ok(header) => header,
                    Err(e) => {
                        error!("failed to unpack header; err = {:?}\nbuf:{:?}", e, &buf);
                        break;
                    }
                };

                if !header.check_flag() {
                    error!("数据头校验失败");
                    break;
                }

                match header.pack_type {
                    PackType::Data => {
                        let mut readed_len = 0;
                        let mut buf = vec![0; header.body_size as usize];
                        while readed_len < header.body_size {
                            let mut tmp_buf =
                                vec![0; header.body_size as usize - readed_len as usize];
                            let n = match stream.read(&mut tmp_buf).await {
                                Ok(n) if n == 0 => {
                                    debug!("连接断开");
                                    return;
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

                        let decoded = match proto::Data::unpack(&buf[..header.body_size as usize]) {
                            Ok(decoded) => decoded,
                            Err(e) => {
                                error!("failed to unpack data; err = {:?}", e);
                                continue;
                            }
                        };
                        trace!(
                            "读取到用户数据：{:?}",
                            String::from_utf8_lossy(&decoded.data)
                        );

                        info!("发送给客户:{},的数据长度:{}", ip_port, decoded.data.len());
                        if let Err(e) = socket.write(&decoded.data).await {
                            error!("转发数据给客户端出错，user={}, err = {:?}", ip_port, e);
                            return;
                        }

                    }
                    PackType::DataEnd => {
                        let mut readed_len = 0;
                        let mut buf = vec![0; header.body_size as usize];
                        while readed_len < header.body_size {
                            let mut tmp_buf =
                                vec![0; header.body_size as usize - readed_len as usize];
                            let n = match stream.read(&mut tmp_buf).await {
                                Ok(n) if n == 0 => {
                                    debug!("连接断开");
                                    return;
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
                        socket.flush().await.unwrap();
                        debug!("数据读取完毕");
                        break;
                    }
                    _ => {
                        error!("未知的数据包类型");
                        break;
                    }
                }
            }
        
            debug!("Finished handling connection from: {}", ip_port);
        });
    }
}
