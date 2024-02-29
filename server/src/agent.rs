use anyhow::bail;
use proto::{PackType, Packet, UnPack};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;
use tracing::{debug, error, trace};

// 用户端
pub struct Agent {
    // 用于接收外部发给自己的数据
    sender: Sender<proto::Packet>,
    // 用于把自己的数据发送给外部
    receiver: Arc<Mutex<Receiver<proto::Packet>>>,
}

impl Agent {
    pub async fn new(ip_port: String, socket: TcpStream) -> anyhow::Result<Self> {
        // receiver_in 接收发进来的数据，所以把sender传出去，给别人发送数据
        let (sender, mut receiver_in) = tokio::sync::mpsc::channel::<proto::Packet>(100);

        // sender_out 发出去的数据，所以把receiver传出去，接收别人的数据
        let (sender_out, receiver) = tokio::sync::mpsc::channel::<proto::Packet>(100);

        let (mut reader, mut writer) = socket.into_split();
        let ip_port_clone = ip_port.clone();
        tokio::spawn(async move {
            loop {
                let packet = read_packet_from_socket(&mut reader, ip_port_clone.clone())
                    .await
                    .unwrap();
                let is_finish = packet.header.pack_type == PackType::DisConnect;

                if sender_out.is_closed() {
                    debug!("发送数据通道已关闭");
                    break;
                }

                if let Err(e) = sender_out.send(packet).await {
                    error!("发送数据失败: {:#?}", e);
                    break;
                }

                if is_finish {
                    break;
                }
            }
        });

        tokio::spawn(async move {
            loop {
                match receiver_in.recv().await {
                    Some(packet) => match packet.header.pack_type {
                        PackType::DisConnect => {
                            debug!("收到关闭连接的消息");
                            break;
                        }
                        PackType::Data => {
                            debug!("收到数据包");
                            let data = proto::Data::unpack(&packet.data).unwrap();
                            if let Err(e) = writer.write_all(&data.data).await {
                                error!("socket发送数据失败: {:?}", e);
                                break;
                            }
                        }
                        _ => {}
                    },
                    None => {
                        break;
                    }
                }
            }
        });

        let agent = Agent {
            sender,
            receiver: Arc::new(Mutex::new(receiver)),
        };

        Ok(agent)
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

async fn read_packet_from_socket(
    reader: &mut tokio::net::tcp::OwnedReadHalf,
    ip_port: String,
) -> anyhow::Result<proto::Packet> {
    // 循环读取http请求信息，直到找到\r\n\r\n
    let mut buf = vec![];
    let header_end;
    let mut header_readed_len = 0;

    // 读取可能需要修改为超时，否则可能会卡死在这里，所有读取都应该改，后面再修改
    // 循环保证读取完整头信息，不然解析会出错
    loop {
        let mut tmp_buf = vec![0; 1024];
        let n = match reader.read(&mut tmp_buf).await {
            Ok(0) => {
                // 读取到0字节，说明对端关闭了连接
                let packet = proto::Packet::new_without_data(proto::PackType::DisConnect);
                return Ok(packet);
            }
            Ok(n) => n,
            Err(e) => {
                error!("读取数据失败: {:?}", e);
                let packet = proto::Packet::new_without_data(proto::PackType::DisConnect);
                return Ok(packet);
            }
        };

        buf.extend_from_slice(&tmp_buf[..n]);

        header_readed_len += n;

        if let Some(index) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
    }

    if header_end == 0 {
        error!("异常的头信息");
        debug!("异常的头信息: {}", String::from_utf8_lossy(&buf));
        bail!("异常的头信息");
    }

    // 获得http头中的 host 字段
    let header = String::from_utf8_lossy(&buf[..header_end]);
    let host = header
        .lines()
        .find(|line| line.starts_with("Host: "))
        .map(|line| line.trim_start_matches("Host: ").to_string())
        .unwrap();

    // 获取content-len长度
    let conent_len = header
        .lines()
        .find(|line| line.starts_with("Content-Length: "))
        .map(|line| {
            line.trim_start_matches("Content-Length: ")
                .parse::<usize>()
                .unwrap()
        });

    // 如果有content-len，就读取body
    if let Some(conent_len) = conent_len {
        // 循环读取body，保证不能多读也不能少读，还要减去上面头信息多读的部分

        // 这是读头信息时多读取的字节数
        let header_more = header_readed_len - header_end;
        // 这是真正还剩下要读取的字节数
        let content_left_len = conent_len - header_more;

        debug!(
            "有body，头大小是：{}， 共读取了：{}， body大小是:{}， 还未读取长度:{}",
            header_end, header_readed_len, conent_len, content_left_len
        );

        let mut body = vec![];
        let mut readed_len = 0;
        while readed_len < content_left_len {
            debug!("进入读取body:{}", readed_len < content_left_len);
            let mut tmp_buf = vec![0; content_left_len - readed_len];
            let n = match reader.read(&mut tmp_buf).await {
                Ok(0) => {
                    // 读取到0字节，说明对端关闭了连接
                    let packet = proto::Packet::new_without_data(proto::PackType::DisConnect);
                    return Ok(packet);
                }
                Ok(n) => n,
                Err(e) => {
                    error!("读取数据失败: {:?}", e);
                    let packet = proto::Packet::new_without_data(proto::PackType::DisConnect);
                    return Ok(packet);
                }
            };
            body.extend_from_slice(&tmp_buf[..n]);
            readed_len += n;
        }

        // 把body和头信息拼接起来
        buf = [&buf[..header_readed_len], &body[..content_left_len]].concat();
    }

    trace!("接收到用户端数据: {}", String::from_utf8_lossy(&buf));

    let data = proto::Data {
        agent_ip_port: ip_port,
        host: Some(host),
        data: buf,
    };

    Ok(Packet::new(PackType::Data, Some(data)).unwrap())
}
