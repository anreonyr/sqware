//! echo::core::line — **正在攒的那一行**：一段字节流按终端约定切成行。
//!
//! 本文件**不碰内核、不碰设备**：它只认"行尾"与"这一行是什么"。本域那几条语义规则全在
//! [`Line::feed`] 一手里——行尾是哪两个字节、CRLF 算几次断行、收场词是哪一个、非 UTF-8 怎么
//! 折；回显那一圈的壳（`adapt/echo.rs`）因此只做"读一批、喂进去、把交出来的行写出去"，连
//! "攒完要清零"都不必记得（账也在 `feed` 里清）。

/// 一行的上界。更长的行**截断**回显（超过它的行不可能是收场词，故收场判据不受影响）；
/// 与设备侧一次排空那一条（`programs/src/driver/uart/main.rs::DRAIN_MAX`）是**同一类跨域约定**
/// ——两端各自写死，靠这一条注对得上。
pub const LINE_MAX: usize = 128;

/// 收场那一个词：读到它 ⇒ 本域退场（域退场 ⇒ 编排域收场 ⇒ 引导域退 ⇒ 停机）。
const EXIT: &[u8] = b"exit";

/// 折不过去的一行写成什么：写的那一路要过 `str`（见 `user/echo/mod.rs` 末一节）。
const NON_UTF8: &str = "<non-utf8>";

/// 喂一个字节之后，这一行走到哪儿了。
///
/// 三格就是终端这一侧的全部出口：**继续攒**、**攒成了一行**、**收场词到了**。
pub enum Fed<'a> {
    /// 还在攒：一个普通字节，或 CRLF 里被吞掉的那一个 `\n`。
    More,
    /// 行尾到了：这一行交出去（借自本行；借出去期间不要再喂）。
    Line(&'a str),
    /// 收场词到了 ⇒ 本域退场。
    Exit,
}

/// 正在攒的那一行：字节一格一格进来，行尾那一格把整行交出去。
pub struct Line {
    raw: [u8; LINE_MAX],
    n: usize,
    /// 上一个字节是 `\r`：它后面紧跟的 `\n` 属于**同一次**断行。
    crlf: bool,
}

impl Line {
    /// 空行：还没攒到任何一格。
    pub fn new() -> Self {
        Self {
            raw: [0; LINE_MAX],
            n: 0,
            crlf: false,
        }
    }

    /// 喂一个字节。**本域的全部判定都在这一手**：行尾、收场词、非 UTF-8 折法、CRLF 算几次
    /// 断行；**账也在这里清**——交出一行之后 `n` 当场归零，故壳里没有"忘了 `clear`"这一步可
    /// 做错。
    ///
    /// 行满之后到的字节**丢掉**（截断回显）：界由 `raw` 的长度承担，不必另判。
    pub fn feed(&mut self, b: u8) -> Fed<'_> {
        // **CRLF = 一次断行**：终端两种断行都认，但认得成一次——`\r` 后面紧跟的 `\n` 是同一次
        // 回车（按 CRLF 发的那一侧否则每行多出一个空行）。
        if b == b'\n' && self.crlf {
            self.crlf = false;
            return Fed::More;
        }
        self.crlf = b == b'\r';
        if b != b'\r' && b != b'\n' {
            if let Some(slot) = self.raw.get_mut(self.n) {
                *slot = b;
                self.n += 1;
            }
            return Fed::More;
        }
        // 行尾：`raw[..n]` 就是这一整行（它只在行尾这一格被读）。**交出去的同时把账清零**——
        // `raw` 的内容不动，清零只是"下一次从第一格开始攒"。
        let n = core::mem::take(&mut self.n);
        if &self.raw[..n] == EXIT {
            return Fed::Exit;
        }
        Fed::Line(core::str::from_utf8(&self.raw[..n]).unwrap_or(NON_UTF8))
    }
}
