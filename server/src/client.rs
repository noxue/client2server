use anyhow::bail;
use clap::Parser;
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


// 客户端
pub struct Client {
    // 指定该客户端要处理哪些host请求
    host: String,
    // 用于接收外部发给自己的数据
    sender: Sender<proto::Packet>,
    // 用于把自己的数据发送给外部
    receiver: Arc<Mutex<Receiver<proto::Packet>>>,
    // 保存线程句柄
    handles: Vec<tokio::task::JoinHandle<()>>,
}

impl Client {
    pub async fn new(host: String, socket: TcpStream) -> anyhow::Result<Self> {
        // receiver_in 接收发进来的数据，所以把sender传出去，给别人发送数据
        let (sender, mut receiver_in) = tokio::sync::mpsc::channel::<proto::Packet>(100);

        // sender_out 发出去的数据，所以把receiver传出去，接收别人的数据
        let (sender_out, receiver) = tokio::sync::mpsc::channel::<proto::Packet>(100);

        let (mut reader, mut writer) = socket.into_split();
        let h1 = tokio::spawn(async move {
            loop {
                let packet = read_packet_from_socket(&mut reader).await.unwrap();
                // 把收到的数据解析成packet，发给app处理
                if let Err(e) = sender_out.send(packet).await {
                    error!("发送数据失败: {:?}", e);
                    break;
                }
            }
        });

        let h2 = tokio::spawn(async move {
            loop {
                match receiver_in.recv().await {
                    Some(packet) => {
                        let data = packet.pack().unwrap();
                        if let Err(e) = writer.write_all(&data).await {
                            error!("发送数据失败: {:?}", e);
                            break;
                        }
                    }
                    None => {
                        break;
                    }
                }
            }
        });

        let client = Client {
            host,
            sender,
            receiver: Arc::new(Mutex::new(receiver)),
            handles: vec![h1, h2],
        };

        Ok(client)
    }

    /// 等待线程任务结束
    pub async fn wait(&mut self) {
        for handle in self.handles.drain(..) {
            if let Err(e) = handle.await {
                error!("线程执行失败: {:?}", e);
            }
        }
    }

    /// 获取发送数据给 Client 的通道
    pub fn sender(&self) -> Sender<proto::Packet> {
        self.sender.clone()
    }

    /// 获取接收 Client 发送的数据的通道
    pub fn receiver(&self) -> Arc<Mutex<Receiver<proto::Packet>>> {
        self.receiver.clone()
    }
}




async fn read_packet_from_socket(
    reader: &mut tokio::net::tcp::OwnedReadHalf,
) -> anyhow::Result<proto::Packet> {
    let header_size = proto::Header::size().unwrap();
    let mut readed_len = 0;
    let mut buf = vec![0; header_size];
    while readed_len < header_size {
        let mut tmp_buf = vec![0; header_size - readed_len];
        let n = match reader.read(&mut tmp_buf).await {
            Ok(n) if n == 0 => {
                debug!("连接断开");

                return Ok(proto::Packet::new_without_data(proto::PackType::DisConnect));
            }
            Ok(n) => n,
            Err(e) => {
                error!("从客户端读取头数据出错; err = {:?}\nbuf:{:?}", e, &buf);
                return Ok(proto::Packet::new_without_data(proto::PackType::DisConnect));
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
            return Ok(proto::Packet::new_without_data(proto::PackType::DisConnect));
        }
    };

    if !header.check_flag() {
        error!("数据头校验失败");
        bail!("数据头校验失败")
    }

    match header.pack_type {
        PackType::Data => {
            let mut readed_len = 0;
            let mut buf = vec![0; header.body_size as usize];
            while readed_len < header.body_size {
                let mut tmp_buf = vec![0; header.body_size as usize - readed_len as usize];
                let n = match reader.read(&mut tmp_buf).await {
                    Ok(n) if n == 0 => {
                        debug!("连接断开");
                        return Ok(proto::Packet::new_without_data(proto::PackType::DisConnect));
                    }
                    Ok(n) => n,
                    Err(e) => {
                        error!("failed to read from socket; err = {:?}", e);
                        bail!("failed to read from socket; err = {:?}", e);
                    }
                };
                buf = [&buf[..readed_len as usize], &tmp_buf[..n]].concat();
                readed_len += n as u64;
            }

            let data = match proto::Data::unpack(&buf[..header.body_size as usize]) {
                Ok(decoded) => decoded,
                Err(e) => {
                    error!("failed to unpack data; err = {:?}", e);
                    bail!("failed to unpack data; err = {:?}", e);
                }
            };

            proto::Packet::new(header.pack_type, Some(data)).map_err(|e| anyhow::anyhow!(e))
        }
        PackType::DataEnd => {
            let mut readed_len = 0;
            let mut buf = vec![0; header.body_size as usize];
            while readed_len < header.body_size {
                let mut tmp_buf = vec![0; header.body_size as usize - readed_len as usize];
                let n = match reader.read(&mut tmp_buf).await {
                    Ok(n) if n == 0 => {
                        debug!("连接断开");
                        return Ok(proto::Packet::new_without_data(proto::PackType::DisConnect));
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
            Ok(proto::Packet::new_without_data(proto::PackType::DataEnd))
        }
        PackType::Heartbeat => {
            debug!("收到心跳包");
            Ok(proto::Packet::new_without_data(proto::PackType::Heartbeat))
        }
        PackType::Login => unimplemented!("登录"),
        PackType::Logout => unimplemented!("登出"),
        PackType::Bind => {
            debug!("绑定host");
            let mut readed_len = 0;
            let mut buf = vec![0; header.body_size as usize];
            while readed_len < header.body_size {
                let mut tmp_buf = vec![0; header.body_size as usize - readed_len as usize];
                let n = match reader.read(&mut tmp_buf).await {
                    Ok(n) if n == 0 => {
                        debug!("连接断开");
                        return Ok(proto::Packet::new_without_data(proto::PackType::DisConnect));
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
            let bind = match proto::Bind::unpack(&buf[..header.body_size as usize]) {
                Ok(decoded) => decoded,
                Err(e) => {
                    error!("failed to unpack data; err = {:?}", e);
                    bail!("failed to unpack data; err = {:?}", e);
                }
            };
            Ok(proto::Packet::new(header.pack_type, Some(bind)).map_err(|e| anyhow::anyhow!(e))?)
        }
        PackType::DisConnect => {
            debug!("主动连接断开");
            return Ok(proto::Packet::new_without_data(proto::PackType::DisConnect));
        }
    }
}
