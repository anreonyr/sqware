//! doom — 他杀（`kill`）的**用户态协议**：一条请求 + 一字节回执。
//!
//! # 机制在核，政策在服务
//!
//! 内核那枚 `RoomCall::Doom` 只认**血缘**（谁生的谁能杀，传递），而 root 是所有域的
//! 祖先 ⇒ 它对任何域的杀都够格。于是"**能不能**"由内核回答、"**该不该**"落在 root
//! 手里——这就是 Linux `kill` 的形状：谁都能请求，够格的那个来执行。内核因此不需要
//! 任何权限表，root 也不需要任何内核入口（`docs/driver.md` §12）。
//!
//! # 线格式
//!
//! ```text
//! Kill：                                      Quit（无回执）：
//! [0]      op      u8        1                [0] op u8   2
//! [1..33)  target  [u8; 32]  `env::Name`
//! [33..41) ack     usize LE  回信孔在**服务侧**的 token
//! ```
//!
//! 名字的账在**目录**里，不在本协议里：目标只按名字点，服务自己去目录解出
//! "这个名字的活实例是谁"。
//!
//! # 回执四值
//!
//! 一字节，**不把内核分好的失败域压成一档**（`Ok`/`Dead`/`Denied`/`Slow`）——
//! 请求方对前两者的反应完全不同（"它已经没了" vs "不许你动它"）。
//!
//! 回信通道**由调用方自带**（报文里那个 `ack`），与目录协议、控制台协议同一条规矩：
//! 谁发起谁备回信通道。

pub mod client;

pub use client::Doom;

use env::wire::NAME_LEN;
use env::{Name, PieToken};

/// 服务的名字（目录里登记的那一个；两端共用一份，不各写一遍）。
pub const SERVICE: &str = "doom";

/// 请求报文长度：`[op u8][target 32][ack 8]`。
pub const REQ_LEN: usize = 1 + NAME_LEN + 8;

/// 回执长度（一字节）。
pub const ACK_LEN: usize = 1;

/// 动词（号段从 1 起，与 `console`/`dispatch` 同一约定：0 留给"无效"）。
///
/// 只有两个：`Kill`（请求杀一个域，41 字节 + 一字节回执）与 `Quit`（**主人停服**，
/// 1 字节、无回执）。后者不是给客户端的：服务线程在本域、是兄弟线程，**不在血缘里**
/// ——级联收不到它，请求孔又是它自己开的 ⇒ 只能由协议说一句"收场"。
pub const OP_KILL: u8 = 1;
pub const OP_QUIT: u8 = 2;

/// 服务处理一条请求时，等目标**消失**的探测间隔（毫秒）与轮次。
///
/// 探的是什么：**服务手里那枚指向目标的副本还在不在**——目标一死，内核的退出钩子会沿
/// 派生链把它（连同目录手里那枚）一起摘掉，那正是"客户端死亡"用的同一条机制
/// （`gate::doom` 的 BFS）。副本没了 = 它走完了死亡路径；探完仍活着 = [`Ack::Slow`]。
pub const GONE_ROUND_MS: usize = 20;
pub const GONE_ROUNDS: usize = 25;

/// 回执四值。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ack {
    /// 已下令，且已等到**它真的没了**：服务手里那枚指向它的副本被摘掉了（派生链随主人
    /// 断——目标走完了死亡路径）。这条承诺是判据的牙：它成立就不只是"收到请求"。
    Ok,
    /// 没有这个目标 / 目标已回收（对上 Linux 的 `ESRCH`）。
    Dead,
    /// 政策不许：请求者不在 root 允许的名单里（对上 `EPERM`）。
    Denied,
    /// 已下令，但它**还在**（在别的核上跑，等 IPI 自退；或它就是不肯让出）——
    /// **不是失败**，只是这一趟没等到。
    Slow,
}

impl Ack {
    pub const fn byte(self) -> u8 {
        match self {
            Ack::Ok => 0,
            Ack::Dead => 1,
            Ack::Denied => 2,
            Ack::Slow => 3,
        }
    }

    /// 一字节 → 值。**未知取值按 `Denied`**：不认识的回执等于"没成"，
    /// 不假装成功。
    pub const fn from_byte(b: u8) -> Ack {
        match b {
            0 => Ack::Ok,
            1 => Ack::Dead,
            2 => Ack::Denied,
            _ => Ack::Slow,
        }
    }
}

/// 一条杀令：按**名字**点目标。
pub struct Kill {
    pub target: Name,
    /// 回信孔在本协议**服务侧**的 token（服务按此推回执）。
    pub ack: PieToken,
}

impl Kill {
    pub const fn new(target: Name, ack: PieToken) -> Kill {
        Kill { target, ack }
    }

    pub fn encode(&self) -> [u8; REQ_LEN] {
        let mut msg = [0u8; REQ_LEN];
        msg[0] = OP_KILL;
        msg[1..1 + NAME_LEN].copy_from_slice(self.target.bytes());
        msg[1 + NAME_LEN..].copy_from_slice(&self.ack.get().to_le_bytes());
        msg
    }

    /// 解码：**长度、动词、名字**三者都要过（非法名不可表达——名字的构造义务在
    /// `Name::from_bytes` 上，这里只是把它接上）。
    pub fn decode(msg: &[u8]) -> Option<Kill> {
        if msg.len() < REQ_LEN || msg[0] != OP_KILL {
            return None;
        }
        let bytes: [u8; NAME_LEN] = msg[1..1 + NAME_LEN].try_into().ok()?;
        let target = Name::from_bytes(bytes).ok()?;
        let ack = usize::from_le_bytes(msg[1 + NAME_LEN..REQ_LEN].try_into().ok()?);
        Some(Kill {
            target,
            ack: PieToken::new(ack),
        })
    }
}
