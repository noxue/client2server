use crate::{client::Client, service::Service};
use proto::{PackType, UnPack};
use std::{collections::HashMap, sync::Arc};
use tokio::{net::TcpStream, sync::Mutex};
use tracing::{debug, error, info};

pub struct App {
    services: Arc<Mutex<HashMap<String, Service>>>,
    client: Option<Arc<Mutex<Client>>>,
}

impl App {
    pub async fn new() -> anyhow::Result<Self> {
        let services = Arc::new(Mutex::new(HashMap::new()));

        Ok(App {
            services,
            client: None,
        })
    }

    pub async fn run(
        &mut self,
        server_ip: String,
        server_port: u16,
        ip: String,
        port: u16,
        host: String,
    ) -> anyhow::Result<()> {
        let server_addr = format!("{}:{}", server_ip, server_port);
        let socket = match TcpStream::connect(&server_addr).await {
            Ok(socket) => socket,
            Err(e) => {
                error!("连接服务器失败: {:?}", e);
                return Err(anyhow::anyhow!("连接服务器失败"));
            }
        };
        info!("连接服务器成功: {:?}", server_addr);

        let client = Client::new(host, socket).await?;
        let client_sender = client.sender().await;
        let client_receiver = client.receiver().await;
        self.client = Some(Arc::new(Mutex::new(client)));
        let services = self.services.clone();
        // 接受client的数据
        let h1 = tokio::spawn(async move {
            loop {
                let packet = client_receiver.lock().await.recv().await;
                if packet.is_none() {
                    break;
                }
                let packet = packet.unwrap();

                match packet.header.pack_type {
                    PackType::Data => {
                        let data = proto::Data::unpack(&packet.data).unwrap();
                        let mut services = services.lock().await;
                        // 如果是新的客户，创建一个服务
                        if !services.contains_key(&data.agent_ip_port) {
                            let service_addr = format!("{}:{}", ip, port);
                            info!("来自 {:?} 的新请求", &data.agent_ip_port);
                            let service_socket = TcpStream::connect(service_addr).await.unwrap();
                            let service = Service::new(data.agent_ip_port.clone(), service_socket)
                                .await
                                .unwrap();
                            // let sender = service.sender().await;
                            let receiver = service.receiver().await;
                            services.insert(data.agent_ip_port.clone(), service);

                            let client_sender = client_sender.clone();
                            tokio::spawn(async move {
                                loop {
                                    let packet = receiver.lock().await.recv().await;
                                    if packet.is_none() {
                                        break;
                                    }
                                    let packet = packet.unwrap();
                                    if let Err(e) = client_sender.send(packet).await {
                                        error!("发送数据失败: {:?}", e);
                                        break;
                                    }
                                }
                            });
                        }
                        if let Some(service) = services.get(&data.agent_ip_port) {
                            let sender = service.sender().await;
                            if let Err(e) = sender.send(packet).await {
                                error!("发送数据失败: {:?}", e);

                                continue;
                            }
                        } else {
                            debug!("没有找到服务");
                        }
                    }

                    _ => {}
                }
            }
        });

        h1.await.unwrap();
        Ok(())
    }
}
