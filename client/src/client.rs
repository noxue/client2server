use anyhow::bail;
use proto::{Pack, PackType, Packet, UnPack};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{
        mpsc::{Receiver, Sender},
        Mutex,
    },
};
use tracing::{debug, error};

pub struct Client {
    sender: Sender<Packet>,
    receiver: Arc<Mutex<Receiver<Packet>>>,
}

impl Client {
    pub async fn new(host: String, socket: TcpStream) -> anyhow::Result<Self> {
        let (sender, mut receiver_in) = tokio::sync::mpsc::channel::<Packet>(100);
        let (sender_out, receiver) = tokio::sync::mpsc::channel::<Packet>(100);
        let (mut reader, mut writer) = socket.into_split();

        // 发送bind
        let packet = Packet::new(
            proto::PackType::Bind,
            Some(proto::Bind { host: host.clone() }),
        )
        .unwrap();
        let encoded = packet.pack().unwrap();
        debug!("发送bind数据包:{:x?}", encoded);
        if let Err(e) = writer.write_all(&encoded).await {
            error!("发送bind数据包失败; err = {:?}", e);
            bail!("发送bind数据包失败");
        }

        tokio::spawn(async move {
            loop {
                let packet = read_packet_from_socket(&mut reader).await.unwrap();
                if let Err(e) = sender_out.send(packet).await {
                    error!("发送数据失败: {:?}", e);
                    break;
                }
            }
        });
        tokio::spawn(async move {
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
            sender,
            receiver: Arc::new(Mutex::new(receiver)),
        };
        Ok(client)
    }

    pub async fn sender(&self) -> Sender<Packet> {
        self.sender.clone()
    }

    pub async fn receiver(&self) -> Arc<Mutex<Receiver<Packet>>> {
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
