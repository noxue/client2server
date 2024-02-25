use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time;
use tracing::{debug, error, info, trace};
use tracing_subscriber::field::debug;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::env::set_var("RUST_LOG", "debug");
    tracing_subscriber::fmt::init();

    let user_listener = TcpListener::bind("0.0.0.0:3000").await?;
    let client_listener = TcpListener::bind("0.0.0.0:4000").await?;

    info!("Server running on port 3000");

    let client_conn = Arc::new(Mutex::new(None));

    let client_conn_clone = client_conn.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, addr) = client_listener.accept().await.unwrap();
            {
                *client_conn_clone.lock().await = Some(socket);
            }
            info!("接收到客户端连接: {}", addr);
        }
    });

    loop {
        let (mut socket, addr) = user_listener.accept().await?;
        let ip_port = addr.to_string();
        debug!("Accepted connection from: {}", ip_port);

        let client_conn = client_conn.clone();
        // 使用Tokio的任务来异步处理每个连接
        tokio::spawn(async move {
            debug!("Handling connection from: {}", ip_port);
            // let socket = binding.get_mut(&ip_port).unwrap();
            let (mut reader, mut writer) = socket.split();
            let mut buf = [0; 102400];

            // 对127.0.0.1:8000 发起连接
            // let mut stream = TcpStream::connect("127.0.0.1:8000").await.unwrap();

            let binding = client_conn.clone();
            let mut binding = binding.lock().await;
            // let stream = binding.as_mut().unwrap();
            let stream = match binding.as_mut() {
                Some(stream) => stream,
                None => {
                    error!("未获取到客户端连接");
                    return;
                }
            };
            debug!("Reading from socket:{}", ip_port);
            // 在一个循环中读取数据，直到连接被关闭
            let n = match time::timeout(Duration::from_secs(3), reader.read(&mut buf)).await {
                Ok(Ok(0)) => {
                    debug!("Connection closed");
                    return;
                }
                Ok(Ok(n)) => n,
                Ok(Err(e)) => {
                    error!("Failed to read from socket: {:?}", e);
                    return;
                }
                Err(_) => {
                    // 超时处理逻辑
                    info!("Read timeout, closing connection");
                    return;
                }
            };

            // 打印接收到的数据
            trace!("{}", String::from_utf8_lossy(&buf[..n]));
            if let Err(e) = stream.write(&buf[..n]).await {
                error!("发送数据给客户端出错：{:?}", e);
            }

            // 马上刷新标准输出
            // std::io::stdout().flush().unwrap();

            // 检查获取到的数据 如果读取到 \r\n\r\n 表示请求头结束
            let header_end_pos =
                if let Some(index) = String::from_utf8_lossy(&buf[..n]).find("\r\n\r\n") {
                    index
                } else {
                    error!("Failed to find end of header");
                    return;
                };

            // 获取http头的Content-Length，然后读取指定长度的数据
            let content_length = if let Some(index) =
                String::from_utf8_lossy(&buf[..header_end_pos]).find("Content-Length: ")
            {
                let start = index + 16;
                let end = String::from_utf8_lossy(&buf[start..header_end_pos])
                    .find("\r\n")
                    .unwrap_or(header_end_pos - start);
                String::from_utf8_lossy(&buf[start..start + end])
                    .parse::<usize>()
                    .unwrap_or(0)
            } else {
                0
            };

            // body限制小于20M
            if content_length > 0 && content_length < 20 * 1024 * 1024 {
                // 第一个数据包可能已经包含了部分body内容，所以需要减去已经读取的部分
                let content_length = content_length - (n - (header_end_pos + 4));
                let mut body = vec![0; content_length];
                if let Err(e) = reader.read_exact(&mut body).await {
                    error!("Failed to read body: {:?}", e);
                    return;
                }
                trace!("{}", String::from_utf8_lossy(&body));
                stream.write(&body).await.unwrap();
            }

            stream.flush().await.unwrap();

            loop {
                // 从steam中读取的返回给用户
                let mut buf = [0; 1024];
                // 这里应该加入数据包类型来知道接收的数据结束了，在协议中定义即可
                let n = match time::timeout(Duration::from_secs(1), stream.read(&mut buf)).await {
                    Ok(Ok(0)) => {
                        debug!("Connection closed");
                        break;
                    }
                    Ok(Ok(n)) => n,
                    Ok(Err(e)) => {
                        error!("Failed to read from socket: {:?}", e);
                        return;
                    }
                    Err(_) => {
                        // 超时处理逻辑
                        error!("获取客户端数据超时");
                        return;
                    }
                };
                // debug!("收到数据:{}", String::from_utf8_lossy(&buf[..n]));
                writer.write(&buf[..n]).await.unwrap();
                writer.flush().await.unwrap();
            }
            // 马上刷新标准输出
            // std::io::stdout().flush().unwrap();
            debug!("Finished handling connection from: {}", ip_port); 
        });
    }
}
