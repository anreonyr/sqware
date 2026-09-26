//! uart::core::batch — **交出去的那一批**：一条**非空**的字节流批次。
//!
//! 本文件**不碰内核、不碰设备**：它只认"这一批有多少内容"。内核那一格（`push`）只收
//! `1..=一页` 的报文 ⇒ **"交 0 字节"写不出来**（不变量做进类型）；这不是丢字节：一批
//! 0 字节本来就没有内容可交（见 `driver/uart/mod.rs` 那条照实记）。

/// 一批要交出去的字节：**空的那一趟不存在**。
pub struct Batch<'a> {
    bytes: &'a [u8],
}

impl<'a> Batch<'a> {
    /// 排空读到 `n` 字节的那一批：`n == 0` ⇒ `None`（这一批没有内容可交）。
    pub fn of(raw: &'a [u8], n: usize) -> Option<Batch<'a>> {
        (n > 0).then(|| Batch { bytes: &raw[..n] })
    }

    /// 这一批的内容（非空由构造保证）。
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}
