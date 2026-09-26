//! echo::core::line — **正在攒的那一行**：一段字节流按终端约定切成行。
//!
//! 本文件**不碰内核、不碰设备**：它只认"行尾"与"这一行是什么"。本域那三条语义规则全在这里
//! ——行尾是哪两个字节、收场词是哪一个、非 UTF-8 怎么折；回显那一圈的壳里因此一条规则也没有
//! （见 `adapt/echo.rs`）。

/// 一行的上界。更长的行**截断**回显（超过它的行不可能是收场词，故收场判据不受影响）；
/// 与设备侧一次排空那一条（`programs/src/driver/uart/adapt/resident.rs::DRAIN_MAX`）是**同一类
/// 跨域约定**——两端各自写死，靠这一条注对得上。
pub const LINE_MAX: usize = 128;

/// 收场那一个词：读到它 ⇒ 本域退场（域退场 ⇒ 编排域收场 ⇒ 引导域退 ⇒ 停机）。
const EXIT: &[u8] = b"exit";

/// 折不过去的一行写成什么：写的那一路要过 `str`（见 `user/echo/mod.rs` 末一节）。
const NON_UTF8: &str = "<non-utf8>";

/// 正在攒的那一行：字节一格一格进来，行尾那一格出去。
pub struct Line {
    raw: [u8; LINE_MAX],
    n: usize,
}

impl Line {
    /// 空行：还没攒到任何一格。
    pub fn new() -> Self {
        Self {
            raw: [0; LINE_MAX],
            n: 0,
        }
    }

    /// 这个字节是行尾吗（`\n` 与 `\r` 都算——终端两种断行都认）。
    pub fn ends(b: u8) -> bool {
        b == b'\n' || b == b'\r'
    }

    /// 攒一格。**行满之后到的字节丢掉**（截断回显）：界由 `raw` 的长度承担，不必另判。
    pub fn put(&mut self, b: u8) {
        if let Some(slot) = self.raw.get_mut(self.n) {
            *slot = b;
            self.n += 1;
        }
    }

    /// 攒起来的这一行是不是收场词。
    pub fn exit(&self) -> bool {
        self.text() == EXIT
    }

    /// 能写出去的那一串：折不过去的折成 [`NON_UTF8`]。
    pub fn word(&self) -> &str {
        core::str::from_utf8(self.text()).unwrap_or(NON_UTF8)
    }

    /// 切完一行，从头再攒。
    pub fn clear(&mut self) {
        self.n = 0;
    }

    /// 已经攒进去的那几格（**只在行尾那一格问**，故它是完整的一行）。
    fn text(&self) -> &[u8] {
        &self.raw[..self.n]
    }
}
