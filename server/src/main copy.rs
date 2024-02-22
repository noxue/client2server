extern crate tokio;

use std::{collections::HashMap, sync::Arc};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

#[tokio::main]
async fn main() {
    // 监听端口

    // 用arc<mutex<hashmap 保存连接
    let conns = Arc::new(Mutex::new(HashMap::new()));
    // 监听端口接收连接，并保存到上面的conns中
    let handle = tokio::spawn(async move {
        // 给本地客户端连接
        let addr = "127.0.0.1:9999";
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        println!("Listening on: {}", addr);
        loop {
            let (mut socket, addr) = listener.accept().await.unwrap();
            println!("Accepted connection from: {}", addr);
            let mut conns = conns.lock().await;
            conns.insert(addr, socket);
        }
    });

    let addr = "127.0.0.1:8888";
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    println!("Listening on: {}", addr);

    loop {
        let (mut socket, addr) = listener.accept().await.unwrap();
        println!("Accepted connection from: {}", addr);

        tokio::spawn(async move {
            let mut buf = [0; 1024];
            loop {
                let n = match socket.read(&mut buf).await {
                    Ok(n) if n == 0 => {
                        println!("Connection closed");
                        return;
                    }
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("failed to read from socket; err = {:?}", e);
                        return;
                    }
                };
                println!("Received {} bytes from: {}\ndata:{}", n, addr, String::from_utf8_lossy(&buf[0..n]));
                if let Err(e) = socket.write_all(&buf[0..n]).await {
                    eprintln!("failed to write to socket; err = {:?}", e);
                    return;
                }
            }
        });
    }
}
