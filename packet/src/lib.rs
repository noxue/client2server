use packet_macro::{Pack, UnPack};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fmt::Debug;

pub trait Pack {
    fn pack(&self) -> Result<Vec<u8>, String>;
}

pub trait UnPack {
    fn unpack(encoded: &[u8]) -> Result<Self, String>
    where
        Self: DeserializeOwned;
}

fn pack<T>(t: &T) -> Result<Vec<u8>, String>
where
    T: Serialize,
{
    match bincode::serialize(&t) {
        Ok(v) => Ok(v),
        Err(e) => Err(format!("{:?}", e)),
    }
}

fn unpack<T>(encoded: &[u8]) -> Result<T, String>
where
    T: DeserializeOwned + PartialEq + Debug,
{
    match bincode::deserialize(encoded) {
        Ok(v) => Ok(v),
        Err(e) => Err(format!("{:?}", e)),
    }
}

// #[derive(Serialize, Deserialize, PartialEq, Debug, Pack, UnPack)]
// pub struct PacketHeader<T>
// {
//     pub flag: u16, // 标志位
//     pub body_size: u64,
//     pub pack_type: T, // 数据包类型
// }

// // 获取数据包头部长度
// pub fn get_header_size<T>() -> usize {

// }

#[derive(Serialize, Deserialize, Default, PartialEq, Debug, Pack, UnPack)]
pub struct Header<T> {
    pub flag: u16, // 标志位
    pub body_size: u64,
    pub pack_type: T, // 数据包类型
}

impl<T> Header<T>
where
    T: Serialize + PartialEq + Debug + Sized + Default,
{
    fn new(body_size: u64, pack_type: T) -> Header<T> {
        Header {
            flag: 0x55aa,
            body_size,
            pack_type,
        }
    }

    // 检测标志位是否正确
    pub fn check_flag(&self) -> bool {
        self.flag == 0x55aa
    }
}

/// 数据头
#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct Packet<T> {
    pub header: Header<T>,
    pub data: Vec<u8>,
}

/// T 一个enum类型，表示数据包类型
impl<T> Packet<T>
where
    T: Serialize + PartialEq + Debug + Default,
{
    pub fn new<D>(data_type: T, data: Option<D>) -> Result<Packet<T>, String>
    where
        D: Pack + UnPack,
    {
        let data = match data {
            Some(data) => data.pack()?,
            None => vec![],
        };
        Ok(Packet {
            header: Header::new(data.len() as u64, data_type),
            data,
        })
    }
}

impl<T> Pack for Packet<T>
where
    T: Serialize + PartialEq + Debug + Default,
{
    // 先打包头部，然后再打包数据，再合并，头部固定大小的
    fn pack(&self) -> Result<Vec<u8>, String> {
        let mut data = self.header.pack()?;
        data.extend_from_slice(&self.data);
        Ok(data)
    }
}

// /// 获取数据
// #[derive(Serialize, Deserialize, PartialEq, Debug, Pack, UnPack)]
// pub struct Task {
//     pub task_id: i32,         // 任务id，随机生成， 如果task_id=0 表示没有任务
//     pub product_name: String, // 产品名
// }

// impl Task {
//     // 根据task_id判断是否有任务
//     pub fn has_task(&self) -> bool {
//         self.task_id != 0
//     }

//     pub fn new(task_id: i32, product_name: String) -> Task {
//         Task {
//             task_id,
//             product_name,
//         }
//     }
// }

// /// worker执行完任务后返回执行结果
// #[derive(Serialize, Deserialize, PartialEq, Debug, Pack, UnPack)]
// pub struct TaskResult {
//     pub task_id: i32,                         // 任务id
//     pub result: Result<i32, TaskResultError>, // 执行结果
// }

// // new
// impl TaskResult {
//     pub fn new(task_id: i32, result: Result<i32, TaskResultError>) -> TaskResult {
//         TaskResult { task_id, result }
//     }
// }

// // 任务结果失败类型
// #[derive(Serialize, Deserialize, PartialEq, Debug, Pack, UnPack)]
// pub enum TaskResultError {
//     AccessDenied,    // 无法访问
//     Banned,          // 被封禁
//     ProductNotFound, // 产品不存在
//     Timeout,         // 超时
// }
