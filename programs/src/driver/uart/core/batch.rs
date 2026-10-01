//! 一条非空的字节流批次。
//! `1..=一页` 的报文 ⇒ **"交 0 字节"写不出来**（不变量做进类型）；这不是丢字节：一批

pub struct Batch<'a> {
    bytes: &'a [u8],
}

impl<'a> Batch<'a> {
    /// 排空读到 `n` 字节的那一批：`n == 0` ⇒ `None`（这一批没有内容可交）
    pub fn of(raw: &'a [u8], n: usize) -> Option<Batch<'a>> {
        (n > 0).then(|| Batch { bytes: &raw[..n] })
    }

    /// 这一批的内容（非空由构造保证）
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}
