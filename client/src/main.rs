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
    task::JoinHandle,
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

    #[clap(short = 'd', long = "domain", value_parser, help = "指定使用的域名")]
    host: String,

    #[clap(
        short,
        long,
        value_parser,
        default_value = "info",
        help = "指定日志级别"
    )]
    log_level: String,
}

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
        let socket = TcpStream::connect(server_addr).await?;
        let client = Client::new(host, socket).await?;
        let client_sender = client.sender().await;
        let client_receiver = client.receiver().await;
        self.client = Some(Arc::new(Mutex::new(client)));
        let services = self.services.clone();
        // 接受client的数据
        let h1 = tokio::spawn(async move {

            loop {
                let packet = client_receiver.lock().await.recv().await;
                if packet.is_none(){
                    break;
                }
                let packet = packet.unwrap();

                match packet.header.pack_type {
                    PackType::Data => {
                        let data = proto::Data::unpack(&packet.data).unwrap();
                        
                        // let services = services.clone();
                        let mut services = services.lock().await;
                        // 如果是新的客户，创建一个服务
                        if !services.contains_key(&data.agent_ip_port) {
                            let service_addr = format!("{}:{}", ip, port);
                            let service_socket = TcpStream::connect(service_addr).await.unwrap();
                            let service = Service::new(data.agent_ip_port.clone(), service_socket).await.unwrap();
                            // let sender = service.sender().await;
                            let receiver = service.receiver().await;
                            services.insert(data.agent_ip_port.clone(), service);
                            
                            let client_sender = client_sender.clone();
                            tokio::spawn(async move {
                                loop {
                                    let packet = receiver.lock().await.recv().await;
                                    if packet.is_none(){
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


#[tokio::main]
async fn main() {
    let args = Args::parse();

    std::env::set_var("RUST_LOG", args.log_level);
    tracing_subscriber::fmt::init();

    let mut app = App::new().await.unwrap();
    app.run(
        args.server_ip,
        args.server_port,
        args.ip,
        args.port,
        args.host,
    ).await.unwrap();
}
