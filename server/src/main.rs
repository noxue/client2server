use std::io::Write;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 监听本地的3000端口
    let listener = TcpListener::bind("0.0.0.0:3000").await?;

    println!("Server running on port 3000");

    // 不断接受连接
    loop {
        let (mut socket, _) = listener.accept().await?;

        // 使用Tokio的任务来异步处理每个连接
        tokio::spawn(async move {
            let mut buf = [0; 10240];

            // 在一个循环中读取数据，直到连接被关闭

            let n = match time::timeout(Duration::from_secs(5), socket.read(&mut buf)).await {
                Ok(Ok(0)) => {
                    println!("Connection closed");
                    return;
                }
                Ok(Ok(n)) => n,
                Ok(Err(e)) => {
                    eprintln!("Failed to read from socket: {:?}", e);
                    return;
                }
                Err(_) => {
                    // 超时处理逻辑
                    eprintln!("Read timeout, closing connection");
                    return;
                }
            };

            // 打印接收到的数据
            print!("{}", String::from_utf8_lossy(&buf[..n]));
            // 马上刷新标准输出
            std::io::stdout().flush().unwrap();

            // 检查获取到的数据 如果读取到 \r\n\r\n 表示请求头结束
            for i in 0..n {
                if i + 3 < n
                    && buf[i] == 13
                    && buf[i + 1] == 10
                    && buf[i + 2] == 13
                    && buf[i + 3] == 10
                {
                    // 获取http头的Content-Length，然后读取指定长度的数据
                    let content_length = if let Some(index) =
                        String::from_utf8_lossy(&buf[..i]).find("Content-Length: ")
                    {
                        let start = index + 16;
                        let end = String::from_utf8_lossy(&buf[start..i])
                            .find("\r\n")
                            .unwrap_or(i - start);
                        String::from_utf8_lossy(&buf[start..start + end])
                            .parse::<usize>()
                            .unwrap_or(0)
                    } else {
                        0
                    };

                    // 小于20M
                    if content_length > 0 && content_length < 20 * 1024 * 1024 {
                        // 第一个数据包可能已经包含了部分body内容，所以需要减去已经读取的部分
                        let content_length = content_length - (n - (i + 4));
                        let mut body = vec![0; content_length];
                        if let Err(e) = socket.read_exact(&mut body).await {
                            eprintln!("Failed to read body: {:?}", e);
                            return;
                        }
                        println!("{}", String::from_utf8_lossy(&body));
                    }

                    println!("读信息结束",);
                    break;
                }
            }

            let html = r#"HTTP/1.1 200 OK
Content-Type: text/html; charset=UTF-8
Content-Length: 15

<h1>Hello World!</h1>"#;

            if let Err(e) = socket.write_all(html.as_bytes()).await {
                eprintln!("Failed to write to socket: {:?}", e);
                return;
            }
        });
    }
}
