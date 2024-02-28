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


// 用户端
pub struct Agent {
    // 表示用户标识，是用户ip:port
    ip_port: String,
    // 用于接收外部发给自己的数据
    sender: Sender<proto::Packet>,
    // 用于把自己的数据发送给外部
    receiver: Arc<Mutex<Receiver<proto::Packet>>>,
    // 保存线程句柄
    handles: Vec<tokio::task::JoinHandle<()>>,
}

impl Agent {
    pub async fn new(ip_port: String, socket: TcpStream) -> anyhow::Result<Self> {
        // receiver_in 接收发进来的数据，所以把sender传出去，给别人发送数据
        let (sender, mut receiver_in) = tokio::sync::mpsc::channel::<proto::Packet>(100);

        // sender_out 发出去的数据，所以把receiver传出去，接收别人的数据
        let (sender_out, receiver) = tokio::sync::mpsc::channel::<proto::Packet>(100);

        let (mut reader, mut writer) = socket.into_split();
        let ip_port_clone = ip_port.clone();
        let h1 = tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) => {
                        // 读取到0字节，说明对端关闭了连接
                        break;
                    }
                    Ok(n) => {
                        let data = buf[..n].to_vec();
                        let data = proto::Data {
                            user_id: ip_port_clone.clone(),
                            data,
                        };
                        let packet = proto::Packet::new(proto::PackType::Data, Some(data)).unwrap();
                        if let Err(e) = sender_out.send(packet).await {
                            error!("发送数据失败: {:?}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        error!("读取数据失败: {:?}", e);
                        break;
                    }
                }
            }
        });

        let h2 = tokio::spawn(async move {
            loop {
                match receiver_in.recv().await {
                    Some(data) => {
                        if let Err(e) = writer.write_all(&data.data).await {
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

        let agent = Agent {
            ip_port,
            sender,
            receiver: Arc::new(Mutex::new(receiver)),
            handles: vec![h1, h2],
        };

        Ok(agent)
    }

    /// 等待线程任务结束
    pub async fn wait(&mut self) {
        for handle in self.handles.drain(..) {
            if let Err(e) = handle.await {
                error!("线程执行失败: {:?}", e);
            }
        }
    }

    /// 获取发送数据给 Agent 的通道
    pub fn sender(&self) -> Sender<proto::Packet> {
        self.sender.clone()
    }

    /// 获取接收 Agent 发送的数据的通道
    pub fn receiver(&self) -> Arc<Mutex<Receiver<proto::Packet>>> {
        self.receiver.clone()
    }
}
