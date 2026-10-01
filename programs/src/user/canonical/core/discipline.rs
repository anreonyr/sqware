//! canonical::core::discipline — **行规程**：canonical mode 的最小一份。
//! UNIX 那一台 tty 的规矩：**回显是终端的事**（不是程序的事），行只在行尾那一格交出去，而
//! ERASE / KILL / EOF 由终端**就地**处理掉。本机没有 tty（`kernel/src/console.rs` 那台只负责把
//! 字节搬进搬出）⇒ 由本域扮演终端。判定全在 [`Discipline::feed`] 一手里，`adapt/` 那一圈因此
//! 没有半条行语义。
//! ```text
//!   canonical  行只在行尾那一格交出去；行编辑对交出去的行不可见
//!   ICRNL      `\r` 当行尾（交互时敲的回车就是它；`\r\n` 只算一次）
//!   ECHO       收到的字符当场回显（写进调用方给的那只 `echo` 缓冲里）
//!   ECHOCTL    控制字符按 `^X` 记（`^C` / `^D` / `^U` 因此看得见）
//!   ERASE      DEL / BS：删掉最后一个**字符**（多字节不拆），回显"退一格盖掉"
//!   KILL       `^U`：丢掉整行
//!   EOF        `^D`：空行上 ⇒ EOF；行上有字 ⇒ 先把这一行交出去（UNIX 就是这两格）
//! ```
//! **不做的**：ISIG（`^C` 不当信号）、IXON、原始模式、`INLCR`、IEXTEN、输出侧 `ONLCR`（输出的
//! `\n` 由终端那一侧落到行首——本机实测就是如此）、光标移动 / 历史 / 按行宽折行。这几格要么与
//! 这台机器无关，要么要一整台信号设施；记在这里是为了别把"没做"当成"做了"。
//! 行满 [`LINE_MAX`] 那一格**截断**（多出的字节丢掉）：界由这一只手自己承担。

use alloc::vec::Vec;

/// 一行的上界。更长的行**截断**。
pub const LINE_MAX: usize = 128;

const EXIT: &[u8] = b"exit";

/// 行规程吃一个字节之后的去向。
pub enum Step {
    /// 继续攒（要回显的字节已经写进 `echo`）。
    More,
    /// 攒成了一行：交出去（**本域没有下游**，故它只到这里为止）。
    Line,
    Exit,
    /// EOF（`^D` 落在空行上）：此后不会再有输入。
    Eof,
}

/// **终端那一侧的状态**（见文件头那张表）。
pub struct Discipline {
    raw: [u8; LINE_MAX],
    n: usize,
    /// 上一次吃进去的是 `\r`：`\r\n` 只算一次断行（那一格 `\n` 不回显、也不再断一次）。
    cr: bool,
}

impl Discipline {
    /// 空行：还没攒到任何一格。
    pub fn new() -> Discipline {
        Discipline {
            raw: [0; LINE_MAX],
            n: 0,
            cr: false,
        }
    }

    /// **行规程的全部判定都在这一手**：ICRNL、ECHO / ECHOCTL、ERASE、KILL、EOF。
    pub fn feed(&mut self, b: u8, echo: &mut Vec<u8>) -> Step {
        if b == b'\n' && self.cr {
            self.cr = false;
            return Step::More;
        }
        self.cr = b == b'\r';
        match b {
            // ICRNL（`\r` 当行尾）＋ ECHO：回显成"回到行首 ＋ 换行"。
            b'\r' | b'\n' => {
                echo.extend_from_slice(b"\r\n");
                self.end()
            }
            // ERASE：DEL / BS —— 删掉最后一个**字符**（多字节不拆），回显"退一格盖掉"。
            0x7f | 0x08 => {
                if self.erase() {
                    echo.extend_from_slice(b"\x08 \x08");
                }
                Step::More
            }
            // KILL：`^U` —— 丢掉整行（ECHOCTL：那一格记出来，再换一行）。
            0x15 => {
                if self.n > 0 {
                    self.n = 0;
                    echo.extend_from_slice(b"^U\r\n");
                }
                Step::More
            }
            // EOF：`^D` —— 行上有字就先把它交出去（UNIX 那两格），空行上才是真的 EOF。
            0x04 => {
                echo.extend_from_slice(b"^D");
                if self.n > 0 { self.end() } else { Step::Eof }
            }
            // 其余控制字符按 ECHOCTL 记成 `^X`（`^C` 之类因此看得见——但**不当信号**，见文件头）。
            _ => {
                if b < 0x20 {
                    echo.push(b'^');
                    echo.push(b + 0x40);
                } else {
                    echo.push(b);
                }
                if let Some(slot) = self.raw.get_mut(self.n) {
                    *slot = b;
                    self.n += 1;
                }
                Step::More
            }
        }
    }

    /// 行尾那一格：把攒下的这一行交出去（**同时清账**）；收场词单独报一格。
    fn end(&mut self) -> Step {
        let exit = &self.raw[..self.n] == EXIT;
        self.n = 0;
        if exit { Step::Exit } else { Step::Line }
    }

    /// 删掉最后一个**字符**：UTF-8 的续字节（`0b10xxxxxx`）一起删。返"删掉了没有"。
    fn erase(&mut self) -> bool {
        if self.n == 0 {
            return false;
        }
        let mut k = self.n - 1;
        while k > 0 && (self.raw[k] & 0xC0) == 0x80 {
            k -= 1;
        }
        self.n = k;
        true
    }
}
