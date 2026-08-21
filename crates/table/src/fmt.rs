//! Fmt —— 行缓冲格式化器：散落逐段直写收拢成「拼一行 → 一次 flush」。
//!
//! 无堆（ArrayString 有界缓冲、写满截断、不 panic），泛 sink（flush 的目标
//! 由调用方每次传入，可 console / trace-host / test，不硬绑底层）。
//! 每格式一个方法（formatter），不抽统一 trait。

use core::fmt::{self, Write};

use crate::hex::render_addr;
use crate::table::Line;

/// 行缓冲格式化器：把格式化的片段拼进一个有界栈行，收行整段直写 sink。
pub struct Fmt<const CAP: usize> {
    buf: Line<CAP>,
}

impl<const CAP: usize> Fmt<CAP> {
    /// 开写一行（空缓冲）。
    pub fn new() -> Self {
        Self { buf: Line::new() }
    }

    /// 追加一个地址（符号 / 分组 hex，判定复用 hex 层，未注入符号器则裸 hex）。
    pub fn addr(&mut self, a: usize) {
        let _ = render_addr(&mut self.buf, a);
    }

    /// 追加裸 hex（{:#x} 具名版）。
    pub fn hex(&mut self, a: usize) {
        let _ = write!(self.buf, "{a:#x}");
    }

    /// 追加字节数（B / KiB / MiB，>=1MiB 才换 MiB）。
    pub fn size(&mut self, n: usize) {
        if n >= 1 << 20 {
            let _ = write!(self.buf, "{:.1} MiB", n as f64 / (1 << 20) as f64);
        } else if n >= 1 << 10 {
            let _ = write!(self.buf, "{:.1} KiB", n as f64 / (1 << 10) as f64);
        } else {
            let _ = write!(self.buf, "{n} B");
        }
    }

    /// 收行：把整行一次写到 out，随后清空缓冲（可复用本 Fmt 拼下一行）。
    pub fn flush<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        let r = out.write_str(self.buf.as_str());
        self.buf.clear();
        r
    }

    /// 读回已写缓冲（测试 / 转存，不消耗）。
    pub fn as_str(&self) -> &str {
        self.buf.as_str()
    }
}

impl<const CAP: usize> Default for Fmt<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAP: usize> Write for Fmt<CAP> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.buf.push_str(s);
        Ok(())
    }
}
