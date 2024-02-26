use proto::{Pack, UnPack};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{broadcast::error, Mutex},
};
use tracing::{debug, error, info, trace};

#[tokio::main]
async fn main() {
    std::env::set_var("RUST_LOG", "debug");
    tracing_subscriber::fmt::init();

    // 对 4000 发起连接

    let mut stream = TcpStream::connect("127.0.0.1:4000").await.unwrap();
    debug!("服务端连接成功");
    //  stream <-> stream2
    loop {
        debug!("正在等待读取用户数据");
        let mut stream2 = match TcpStream::connect("127.0.0.1:8000").await {
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
            let mut buf = vec![0; header_size];
            let n = match stream.read(&mut buf).await {
                Ok(n) if n == 0 => {
                    debug!("连接断开");
                    return;
                }
                Ok(n) => n,
                Err(e) => {
                    error!("failed to read from socket; err = {:?}", e);
                    break;
                }
            };

            let header = match proto::Header::unpack(&buf) {
                Ok(header) => header,
                Err(e) => {
                    error!("failed to unpack header; err = {:?}", e);
                    break;
                }
            };

            match header.pack_type {
                proto::PackType::Data => {
                    let mut buf = vec![0; header.body_size as usize];
                    let n = match stream.read(&mut buf).await {
                        Ok(n) if n == 0 => {
                            debug!("连接断开");
                            return;
                        }
                        Ok(n) => n,
                        Err(e) => {
                            error!("failed to read from socket; err = {:?}", e);
                            return;
                        }
                    };
                    let data = match proto::Data::unpack(&buf) {
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
                    let mut buf = vec![0; header.body_size as usize];
                    let n = match stream.read(&mut buf).await {
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
                    let decoded = match proto::Data::unpack(&buf) {
                        Ok(decoded) => decoded,
                        Err(e) => {
                            error!("failed to unpack data; err = {:?}", e);
                            continue;
                        }
                    };
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
            let mut buf2 = [0; 1024];
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

                    if let Err(e) = stream.write(&encoded).await {
                        error!("把客户端数据转发给用户出错; err = {:?}", e);
                        return;
                    }

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

                    if let Err(e) = stream.write(&encoded).await {
                        error!("把客户端数据转发给用户出错; err = {:?}", e);
                        return;
                    }
                    break;
                }
            };
            trace!(
                "从客户端读取数据完毕，转发给用户:{:?}",
                String::from_utf8_lossy(&buf2[..n])
            );

            let packet = proto::Packet::new(
                proto::PackType::Data,
                Some(proto::Data {
                    user_id: user_id.clone(),
                    data: buf2[..n].to_vec(),
                }),
            )
            .unwrap();

            let encoded = packet.pack().unwrap();

            if let Err(e) = stream.write(&encoded).await {
                error!("把客户端数据转发给用户出错; err = {:?}", e);
                return;
            }
            debug!("成功把客户端的数据转发给用户");
        }
    }
}
