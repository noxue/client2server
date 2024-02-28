use bincode::Options;
pub use packet::{Pack, Packed, UnPack, UnPacked};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize, Packed, UnPacked)]
pub enum PackType {
    Heartbeat,
    Login,
    Logout,
    Data,
    DataEnd,
    Bind, // 表示绑定处理哪个域名下的数据
    DisConnect,
}

impl Default for PackType {
    fn default() -> Self {
        PackType::Heartbeat
    }
}

/************************************
*    用户1----                      *
*            |                      *
*            v                      *
*    用户2---->服务端<----本地客户端  *
*            ^                      *
*            |                      *
*   用户3-----                      *
*************************************
* 上面箭头表示链接的方向，都是主动像服务端发起连接
* 由于本地客户端是主动发起连接，所以无法像服务器那样每个客户都有单独的连接可以区分，所以需要user_id，用来区分
* 这个区分，主要是 本地客户端 返回n个数据包，服务器要知道哪个包应该返回给哪个用户，这就是user_id的作用
**************************************************/
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Packed, UnPacked)]
pub struct Data {
    pub user_id: String, // 外网客户端id，用ip和端口号组成
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Packed, UnPacked)]
pub struct Bind {
    host: String,
}

pub type Packet = packet::Packet<PackType>;
pub type Header = packet::Header<PackType>;
