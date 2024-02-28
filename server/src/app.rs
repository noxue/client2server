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
use tracing::{debug, error, info, trace, warn};

use crate::agent::Agent;
use crate::client::Client;

pub struct App {
    // 用于存储用户端的连接
    agents: Arc<Mutex<HashMap<String, Agent>>>,
    // 用于存储客户端的连接
    clients: Arc<Mutex<HashMap<String, Client>>>,
    // 保存host和客户端的映射关系，key是host，value是客户端ip:port，也就是clients的key
    host_clients: Arc<Mutex<HashMap<String, String>>>,
}

// 启动，监听用户端口和客户端端口
impl App {
    pub async fn new() -> anyhow::Result<Self> {
        let agents = Arc::new(Mutex::new(HashMap::new()));
        let clients = Arc::new(Mutex::new(HashMap::new()));
        let host_clients = Arc::new(Mutex::new(HashMap::new()));
        Ok(App {
            agents,
            clients,
            host_clients,
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
        let host_clients = self.host_clients.clone();
        let clients_clone = self.clients.clone();
        let host_clients_clone = self.host_clients.clone();
        let h1 = tokio::spawn(async move {
            loop {
                match user_listener.accept().await {
                    Ok((socket, addr)) => {
                        let agents = agents.clone();
                        let clients = clients_clone.clone();
                        let host_clients = host_clients_clone.clone();
                        tokio::spawn(async move {
                            if let Err(e) =
                                App::handle_agent(agents, clients, host_clients, socket, addr).await
                            {
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
                        let host_clients = host_clients.clone();
                        tokio::spawn(async move {
                            if let Err(e) =
                                App::handle_client(agents, clients, host_clients, socket, addr)
                                    .await
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
        host_clients: Arc<Mutex<HashMap<String, String>>>,
        socket: TcpStream,
        addr: std::net::SocketAddr,
    ) -> anyhow::Result<()> {
        let agent = Agent::new(addr.to_string(), socket).await?;

        let receiver = agent.receiver();

        {
            let mut agents = agents.lock().await;
            agents.insert(addr.to_string(), agent);
        }

        // 接收用户端发来的数据
        tokio::spawn(async move {
            loop {
                match receiver.lock().await.recv().await {
                    Some(packet) => {
                        // 解包
                        match packet.header.pack_type {
                            PackType::Data => {
                                let data = proto::Data::unpack(&packet.data).unwrap();
                                trace!("接收到用户端数据: {:?}", data);
                                let host_clients = host_clients.lock().await;
                                let ip_port = match host_clients
                                    .get(&data.host.clone().unwrap_or_default())
                                {
                                    Some(ip_port) => ip_port,
                                    None => {
                                        debug!("未找到host对应的客户端: {:?}", data.host);
                                        // 移除agent
                                        let mut agents = agents.lock().await;
                                        // 发送html给用户端
                                        // 提示用户，客户端未启动
                                        let html = r#"<!DOCTYPE html>
                                        <html lang="en">
                                        <head>
                                            <meta charset="UTF-8">
                                            <meta http-equiv="X-UA-Compatible" content="IE=edge">
                                            <meta name="viewport" content="width=device-width, initial-scale=1.0">
                                            <title>提示</title>
                                        </head>
                                        <body>
                                            <h1>客户端未启动</h1>
                                        </body>
                                        </html>"#;
                                        let http_html = format!(
                                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                                            html.len(),
                                            html
                                        );

                                        let packet = proto::Packet::new(
                                            PackType::Data,
                                            Some(proto::Data {
                                                agent_ip_port: addr.to_string(),
                                                host: None,
                                                data: http_html.as_bytes().to_vec(),
                                            }),
                                        )
                                        .unwrap();
                                        if let Err(e) = agents
                                            .get(&data.agent_ip_port)
                                            .unwrap()
                                            .sender()
                                            .send(packet)
                                            .await
                                        {
                                            error!("发送数据给用户端失败: {:?}", e);
                                        }
                                        agents.remove(&addr.to_string());
                                        break;
                                    }
                                };

                                if let Err(e) = clients
                                    .lock()
                                    .await
                                    .get(ip_port)
                                    .unwrap()
                                    .sender()
                                    .send(packet)
                                    .await
                                {
                                    error!("发送数据给客户端失败: {:?}", e);
                                }
                            }
                            PackType::Bind => {
                                let bind = proto::Bind::unpack(&packet.data).unwrap();
                                debug!("接收到用户端绑定请求: {:?}", bind);
                                let mut host_clients = host_clients.lock().await;
                                host_clients.insert(bind.host, addr.to_string());
                            }

                            PackType::DisConnect => {
                                let mut agents = agents.lock().await;
                                agents.remove(&addr.to_string());
                                break;
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

    async fn handle_client(
        agents: Arc<Mutex<HashMap<String, Agent>>>,
        clients: Arc<Mutex<HashMap<String, Client>>>,
        host_clients: Arc<Mutex<HashMap<String, String>>>,
        socket: TcpStream,
        addr: std::net::SocketAddr,
    ) -> anyhow::Result<()> {
        let client = Client::new(addr.to_string(), socket).await?;
        let receiver = client.receiver();

        {
            let mut clients = clients.lock().await;

            clients.insert(addr.to_string(), client);
        }

        // 接收客户端发来的数据
        tokio::spawn(async move {
            loop {
                let packet = receiver.lock().await.recv().await;
                if packet.is_none() {
                    continue;
                }
                let packet = packet.unwrap();
                // 解包
                match packet.header.pack_type {
                    PackType::Data => {
                        let data = proto::Data::unpack(&packet.data).unwrap();
                        debug!("接收到客户端数据: {:?}", data);
                        let agents = agents.lock().await;
                        let agent = match agents.get(&data.agent_ip_port) {
                            Some(agent) => agent,
                            None => {
                                warn!("未找到agent: {:?}", data.agent_ip_port);
                                continue;
                            }
                        };
                        if let Err(e) = agent.sender().send(packet).await {
                            error!("发送数据给用户端失败: {:?}", e);
                        }
                    }
                    PackType::Bind => {
                        let bind = proto::Bind::unpack(&packet.data).unwrap();
                        debug!("接收到客户端绑定请求: {:?}", bind);
                        let mut host_clients = host_clients.lock().await;
                        host_clients.insert(bind.host, addr.to_string());
                    }
                    PackType::DisConnect => {
                        let mut clients = clients.lock().await;
                        clients.remove(&addr.to_string());
                        break;
                    }
                    _ => {
                        error!("未知的数据类型: {:?}", packet.header.pack_type);
                    }
                }
            }
        });

        Ok(())
    }
}
