pub use packet_macro::{Packed, UnPacked};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fmt::Debug;
use bincode::Options;

pub trait Pack {
    fn pack(&self) -> Result<Vec<u8>, String>;
}

pub trait UnPack {
    fn unpack(encoded: &[u8]) -> Result<Self, String>
    where
        Self: DeserializeOwned;
}

#[derive(Serialize, Deserialize, PartialEq, Default, Debug, Packed, UnPacked)]
pub struct Header<T> {
    flag: u16, // 标志位
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

    // 获取数据包头部长度，首先读取头部，然后序列化，然后获取 body 长度
    pub fn size() -> Result<usize, String>
    where
        T: Serialize + PartialEq + Debug + Sized + Default,
    {
        let header = Header::<T>::default();
        Ok(header.pack()?.len())
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

    pub fn new_without_data(data_type: T) -> Packet<T> {
        Packet {
            header: Header::new(0, data_type),
            data: vec![],
        }
    }
}

impl<T> Pack for Packet<T>
where
    T: Serialize + PartialEq + Debug + Default,
{
    // 先打包头部，然后再打包数据，再合并，头部固定大小的
    fn pack(&self) -> Result<Vec<u8>, String> {
        let mut data = self.header.pack()?;
        data.extend_from_slice(&self.data[..self.header.body_size as usize]);
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, PartialEq, Debug, Packed, UnPacked)]
    enum PackType {
        Task,
        TaskResult,
    }

    impl Default for PackType {
        fn default() -> Self {
            PackType::Task
        }
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug, Packed, UnPacked)]
    pub struct Task {
        pub task_id: i32,         // 任务id，随机生成， 如果task_id=0 表示没有任务
        pub product_name: String, // 产品名
    }

    #[test]
    fn test_pack_unpack() {
        let task = Task {
            task_id: 1,
            product_name: "test".to_string(),
        };
        let packet = Packet::new(PackType::Task, Some(task)).unwrap();
        let data = packet.pack().unwrap();
        println!("packed:{:x?}", data);
        // 读取头部
        let header_size = Header::<PackType>::size().unwrap();
        let header = Header::<PackType>::unpack(&data[0..header_size]).unwrap();
        assert_eq!(header.pack_type, PackType::Task);
        assert!(header.check_flag());
        // 读取数据
        let task = Task::unpack(&data[header_size..]).unwrap();
        assert_eq!(task.task_id, 1);
        assert_eq!(task.product_name, "test".to_string());
    }
}
