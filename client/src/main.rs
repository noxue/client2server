use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{broadcast::error, Mutex},
};
use tracing::{debug, error, info, trace};

#[tokio::main]
async fn main() {
    std::env::set_var("RUST_LOG", "debug");
    tracing_subscriber::fmt::init();

    // 对 4000 发起连接
    let mut stream = TcpStream::connect("127.0.0.1:4000").await.unwrap();
    debug!("服务端连接成功");
   

    // let (mut reader, mut writer) = stream.split();
    // let (mut reader2, mut writer2) = stream2.lock().await.as_mut().unwrap().split();

    //  stream <-> stream2
    loop {
        debug!("正在等待读取用户数据");
        let mut buf = [0; 1024];
        let n = match stream.read(&mut buf).await {
            Ok(n) if n == 0 => {
                debug!("连接断开");
                return;
            }
            Ok(n) => n,
            Err(e) => {
                error!("failed to read from socket; err = {:?}", e);
                continue;
            }
        };

        let mut stream2 = match TcpStream::connect("127.0.0.1:8000").await {
            Ok(stream) => {
                debug!("客户端连接成功");
                stream
            }
            Err(e) => {
                error!("连接失败; err = {:?}", e);
                continue;
            }
        };

        debug!("读取到用户数据：{:?}", String::from_utf8_lossy(&buf[..n]));

        debug!("把数据转发给客户端");

        if let Err(e) = stream2.write_all(&buf[0..n]).await {
            error!("转发数据给客户端出错，err = {:?}", e);
            continue;
        }

        loop {
            debug!("已把数据转发给客户端了,等待客户端返回数据");
            let mut buf2 = [0; 102400];
            let n = match stream2.read(&mut buf2).await {
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
            debug!(
                "从客户端读取数据完毕，转发给用户:{:?}",
                String::from_utf8_lossy(&buf2[..n])
            );
            if let Err(e) = stream.write_all(&buf2[0..n]).await {
                error!("把客户端数据转发给用户出错; err = {:?}", e);
                return;
            }
            debug!("成功把客户端的数据转发给用户");
        }
    }
}
