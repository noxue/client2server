use clap::Parser;
use proto::{Pack, UnPack};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tracing::{debug, error, info, trace};

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

    // 对 4000 发起连接
    let mut stream = TcpStream::connect(format!("{}:{}", args.server_ip, args.server_port))
        .await
        .unwrap();
    // let mut stream = TcpStream::connect("121.40.67.145:4000").await.unwrap();
    debug!("服务端连接成功");
    //  stream <-> stream2
    loop {
        debug!("正在等待读取用户数据");
        let mut stream2 = match TcpStream::connect(format!("{}:{}", args.ip, args.port)).await {
            Ok(stream) => {
                debug!("客户端连接成功");
                stream
            }
            Err(e) => {
                error!("连接失败; err = {:?}", e);
                continue;
            }
        };
        let mut user_id = "".to_string();
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

            debug!("收到的打包后的数据:{:x?}", &buf[..header_size]);

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
                proto::PackType::Data => {
                    let mut readed_len = 0;
                    let mut buf = vec![0; header.body_size as usize];
                    while readed_len < header.body_size {
                        let mut tmp_buf = vec![0; header.body_size as usize - readed_len as usize];
                        let n = match stream.read(&mut tmp_buf).await {
                            Ok(n) if n == 0 => {
                                debug!("连接断开");
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

                    let data = match proto::Data::unpack(&buf[..header.body_size as usize]) {
                        Ok(decoded) => decoded,
                        Err(e) => {
                            error!("failed to unpack data; err = {:?}", e);
                            continue;
                        }
                    };
                    user_id = data.user_id;

                    trace!("读取到用户数据：{:?}", String::from_utf8_lossy(&data.data));

                    debug!("把数据转发给客户端");

                    if let Err(e) = stream2.write_all(&data.data).await {
                        error!("转发数据给客户端出错，err = {:?}", e);
                        continue;
                    }
                }
                proto::PackType::DataEnd => {
                    let mut readed_len = 0;
                    let mut buf = vec![0; header.body_size as usize];
                    while readed_len < header.body_size {
                        let mut tmp_buf = vec![0; header.body_size as usize - readed_len as usize];
                        let n = match stream.read(&mut tmp_buf).await {
                            Ok(n) if n == 0 => {
                                debug!("连接断开");
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

                    debug!("数据读取完毕");
                    break;
                }
                _ => {
                    error!("未知的数据包类型");
                    break;
                }
            }
        }

        
        loop {
            debug!("已把数据转发给客户端了,等待客户端返回数据");
            let mut buf2 = vec![0; 5000];
            let n = match stream2.read(&mut buf2).await {
                Ok(n) if n == 0 => {
                    debug!("连接断开");

                    let packet = proto::Packet::new(
                        proto::PackType::DataEnd,
                        Some(proto::Data {
                            user_id: user_id.clone(),
                            data: vec![],
                        }),
                    )
                    .unwrap();
                    let encoded = packet.pack().unwrap();

                    debug!("打包后的数据：{:x?}", encoded);
                    if let Err(e) = stream.write(&encoded).await {
                        error!("把客户端数据转发给用户出错; err = {:?}", e);
                        break;
                    }

                    // fflush
                    stream.flush().await.unwrap();
                    break;
                }
                Ok(n) => n,
                Err(e) => {
                    error!("从客户端读取数据出错; err = {:?}", e);
                    let packet = proto::Packet::new(
                        proto::PackType::DataEnd,
                        Some(proto::Data {
                            user_id: user_id.clone(),
                            data: vec![],
                        }),
                    )
                    .unwrap();
                    let encoded = packet.pack().unwrap();

                    debug!("打包后的数据：{:x?}", encoded);
                    if let Err(e) = stream.write(&encoded).await {
                        error!("把客户端数据转发给用户出错; err = {:?}", e);
                        break;
                    }
                    break;
                }
            };
            trace!(
                "从客户端读取数据完毕，转发给用户:{:?}",
                String::from_utf8_lossy(&buf2[..n])
            );

            info!("读取到的数据长度：{}", n);

            let packet = proto::Packet::new(
                proto::PackType::Data,
                Some(proto::Data {
                    user_id: user_id.clone(),
                    data: buf2[..n].to_vec(),
                }),
            )
            .unwrap();

            let encoded = packet.pack().unwrap();

            debug!("打包后的数据：{:x?}", encoded);
            if let Err(e) = stream.write(&encoded).await {
                error!("把客户端数据转发给用户出错; err = {:?}", e);
                return;
            }
            debug!("成功把客户端的数据转发给用户");
        }
    }
}
