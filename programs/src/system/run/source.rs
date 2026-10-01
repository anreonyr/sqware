//! 这一台身子的那一段字节从哪本账里取。
//! 它只有一条路：**按名字在那块账里取那一段**（零拷贝：它就是那块字节的一个切片）。

use alloc::string::String;

use crate::boot::Catalog;

pub struct Source {
    /// 这一本账的读面。
    catalog: Catalog<'static>,
}

impl Source {
    /// **那一本账**：boot 给的那块（两块账）（清单与全部镜像都在里面，零拷贝借映）。
    pub const fn initrd(catalog: Catalog<'static>) -> Source {
        Source { catalog }
    }

    /// 唯一消费者是装配时那一行 service::mint——它要的正是"一段 `&[u8]`"，别的（名字怎么解析、
    pub fn image(&self, name: String) -> Option<&'static [u8]> {
        self.catalog.find(name.as_str()).map(|entry| entry.elf)
    }
}
