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

use crate::agent::Agent;
use crate::client::Client;

pub struct App {
    // 用于存储用户端的连接
    agents: Arc<Mutex<HashMap<String, Agent>>>,
    // 用于存储客户端的连接
    clients: Arc<Mutex<HashMap<String, Client>>>,
    // 保存host和客户端的映射关系，key是host，value是客户端ip:port，也就是clients的key
    host_client: Arc<Mutex<HashMap<String, String>>>,
}

// 启动，监听用户端口和客户端端口
impl App {
    pub async fn new() -> anyhow::Result<Self> {
        let agents = Arc::new(Mutex::new(HashMap::new()));
        let clients = Arc::new(Mutex::new(HashMap::new()));
        let host_client = Arc::new(Mutex::new(HashMap::new()));
        Ok(App {
            agents,
            clients,
            host_client,
        })
    }

    pub async fn run(
        &self,
        server_ip: &str,
        user_port: u16,
        client_port: u16,
    ) -> anyhow::Result<()> {
        let user_listener = TcpListener::bind(format!("{}:{}", server_ip, user_port)).await?;
        let client_listener = TcpListener::bind(format!("{}:{}", server_ip, client_port)).await?;

        info!(
            "服务端启动成功，监听地址: {}，用户端口：{},  客户端端口：{}",
            server_ip, user_port, client_port
        );

        let agents = self.agents.clone();
        let clients = self.clients.clone();
        let agents_clone = self.agents.clone();
        let clients_clone = self.clients.clone();

        let h1 = tokio::spawn(async move {
            loop {
                match user_listener.accept().await {
                    Ok((socket, addr)) => {
                        let agents = agents.clone();
                        let clients = clients_clone.clone();
                        tokio::spawn(async move {
                            if let Err(e) = App::handle_agent(agents, clients, socket, addr).await {
                                error!("处理用户端连接失败: {:?}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("接收用户端连接失败: {:?}", e);
                    }
                }
            }
        });

        let h2 = tokio::spawn(async move {
            loop {
                match client_listener.accept().await {
                    Ok((socket, addr)) => {
                        let clients = clients.clone();
                        let agents = agents_clone.clone();
                        tokio::spawn(async move {
                            if let Err(e) = App::handle_client(agents, clients, socket, addr).await
                            {
                                error!("处理客户端连接失败: {:?}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("接收客户端连接失败: {:?}", e);
                    }
                }
            }
        });

        h1.await.unwrap();
        h2.await.unwrap();

        Ok(())
    }

    async fn handle_agent(
        agents: Arc<Mutex<HashMap<String, Agent>>>,
        clients: Arc<Mutex<HashMap<String, Client>>>,
        socket: TcpStream,
        addr: std::net::SocketAddr,
    ) -> anyhow::Result<()> {
        let mut agents = agents.lock().await;
        let agent = Agent::new(addr.to_string(), socket).await?;
        agents.insert(addr.to_string(), agent);

        Ok(())
    }

    async fn handle_client(
        agents: Arc<Mutex<HashMap<String, Agent>>>,
        clients: Arc<Mutex<HashMap<String, Client>>>,
        socket: TcpStream,
        addr: std::net::SocketAddr,
    ) -> anyhow::Result<()> {
        let client = Client::new(addr.to_string(), socket).await?;
        let sender = client.sender();
        let receiver = client.receiver();

        {
            let mut clients = clients.lock().await;

            clients.insert(addr.to_string(), client);
        }

        // 接收客户端发来的数据
        tokio::spawn(async move {
            loop {
                match receiver.lock().await.recv().await {
                    Some(packet) => {
                        // 解包
                        match packet.header.pack_type {
                            PackType::Data => {
                                let data = proto::Data::unpack(&packet.data).unwrap();
                                debug!("接收到客户端数据: {:?}", data);
                                if let Err(e) = agents.lock().await.get(&data.user_id).unwrap().sender().send(packet).await{
                                    error!("发送数据给用户端失败: {:?}", e);
                                }
                            }
                            PackType::Bind => {

                            }
                            _ => {
                                error!("未知的数据类型: {:?}", packet.header.pack_type);
                            }
                        }
                    }
                    None => {
                        break;
                    }
                }
            }
        });

        Ok(())
    }
}
