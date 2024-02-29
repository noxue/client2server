use anyhow::bail;
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

pub struct Service {
    sender: Sender<Packet>,
    receiver: Arc<Mutex<Receiver<Packet>>>,
}

impl Service {
    pub async fn new(ip_port: String, socket: TcpStream) -> anyhow::Result<Self> {
        let (sender, mut receiver_in) = tokio::sync::mpsc::channel::<Packet>(100);
        let (sender_out, receiver) = tokio::sync::mpsc::channel::<Packet>(100);
        let (mut reader, mut writer) = socket.into_split();

        // reader收到本地服务返回的http数据，就打包发送出去，最终会发到中转服务器上
        tokio::spawn(async move {
            loop {
                let mut buf = vec![0; 1024];
                let n = match reader.read(&mut buf).await {
                    Ok(n) if n == 0 => {
                        debug!("连接断开");
                        break;
                    }
                    Ok(n) => n,
                    Err(e) => {
                        error!("从客户端读取数据出错; err = {:?}", e);
                        break;
                    }
                };
                let packet = Packet::new(
                    proto::PackType::Data,
                    Some(proto::Data {
                        agent_ip_port: ip_port.clone(),
                        data: buf[..n].to_vec(),
                        host: None,
                    }),
                )
                .unwrap();
                debug!("发送数据包:{:x?}", &packet.pack().unwrap());
                if let Err(e) = sender_out.send(packet).await {
                    error!("发送数据失败: {:?}", e);
                    break;
                }
            }
        });

        // 处理收到的数据包，解包发给本地服务
        tokio::spawn(async move {
            loop {
                match receiver_in.recv().await {
                    Some(packet) => match packet.header.pack_type {
                        PackType::Data => {
                            let data = proto::Data::unpack(&packet.data).unwrap();
                            if let Err(e) = writer.write_all(&data.data).await {
                                error!("发送数据失败: {:?}", e);
                                break;
                            }
                        }
                        PackType::DisConnect => {
                            debug!("连接断开");
                            break;
                        }
                        _ => {}
                    },
                    None => {
                        break;
                    }
                }
            }
        });
        let service = Service {
            sender,
            receiver: Arc::new(Mutex::new(receiver)),
        };
        Ok(service)
    }

    pub async fn sender(&self) -> Sender<Packet> {
        self.sender.clone()
    }

    pub async fn receiver(&self) -> Arc<Mutex<Receiver<Packet>>> {
        self.receiver.clone()
    }
}
