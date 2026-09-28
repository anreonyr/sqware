//! operator::grant —— **操作面那一维**：一枚 `Grant` = 一枚操作。
//!
//! ```text
//!   part 分   land 落   find 寻   trim 剪   list 列   seek 译   name 名      ← 七条原语
//!   Face 一面持树者（全操作面）   Grant 一柄授面的权（**一枚 = 一枚操作**）
//! ```
//!
//! 三位一体一句话：**来源 = 树上那一格，判别 = 会话说的是哪一位**。故
//!
//! - **不可伪造**：请求里没有"我是哪一位"这一格；面在**会话入口的记号**上（`ask()`），
//!   服务端从**它自己表里那枚问话孔**读回（[`grant_of`]）⇒ 客户端无从自称；
//! - **不替代** [`Rule`](super::Rule) / `Owner`：它只判"这一位许不许这一类"，过了之后
//!   原来那两道门照走（`land` 仍是"面 ✓ ＋ 那一格 owner ✓"两层）；
//! - **不上帧**：本文件一个字节都不进报文——线格式、动作码、失败域**一处未动**。
//!
//! # 位次与记号
//!
//! [`Grant::ALL`] 的**次序就是位次 1..=7**（第 i 位 = `ALL[i-1]`），故位次与枚举变体
//! 不会各说各的。记号 = `"operator-ask-" + 面名`：它同时是**那一格入口 Pie 的记号**与
//! **那一面会话说问话孔的记号**（同一个面名，一处给）。
//!
//! **照实记（`0` 是哨兵）**：[`Grant::of_wire`] 对**表外的动作**答 `0`（第八条
//! [`opens`](super::core::Operator::opens) 只读、不上线，故不在表内）——`0` 不是任何一位，
//! 故它**从不参与判面**（判面那一句见 [`Grant::at`]）。
//!
//! # 名字为什么是 `Grant`
//!
//! 一名两读，一眼兼两意：**这一面的「一位」**（位次）与**授出去的「一柄权」**。故不必为
//! 客户端与服务端各造一个词——客户端那具柄叫 [`Rein`](super::client::Rein)（与 `Face` 对偶），
//! 服务端判的那一句就叫 [`Grant::at`]。

use env::Mark;

use super::frame::Wire;
use super::{ASK_MARK, TIP_MARK};

/// **一柄授面的权**：一枚 = 一枚操作。七位，位次 1..=7。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grant {
    /// `part` 分。
    Part,
    /// `land` 落。
    Land,
    /// `find` 寻。
    Find,
    /// `trim` 剪。
    Trim,
    /// `list` 列。
    List,
    /// `seek` 译。
    Seek,
    /// `name` 名。
    Name,
}

impl Grant {
    /// 七位，**次序即位次**（第 i 位在 `ALL[i-1]`）。
    pub const ALL: [Grant; 7] = [
        Grant::Part,
        Grant::Land,
        Grant::Find,
        Grant::Trim,
        Grant::List,
        Grant::Seek,
        Grant::Name,
    ];

    /// **这一位是几**（1..=7）——判面那一句比的就是它。
    ///
    /// 次序由 [`Grant::ALL`] 给，故这里不另写一张表：改枚举次序那条路根本不存在。
    pub const fn at(self) -> u8 {
        let mut i = 0;
        while i < 7 {
            if Grant::ALL[i] as u8 == self as u8 {
                return (i + 1) as u8;
            }
            i += 1;
        }
        // 不可能：每一位都在 `ALL` 里（加变体不补 `ALL` ⇒ 这条循环走到底）。
        panic!("operator: grant not in ALL")
    }

    /// **这一位在哪一条线上动作上答话**——判面时由它拿"请求是第几位"。
    ///
    /// 穷尽 `match`：加一条线上动作不补这里 ⇒ 编不过。`Road` 就是 `seek`
    /// （路那一形只到这一格，见 [`Req`](super::Req)）。
    pub const fn of_wire(wire: &Wire) -> u8 {
        match wire {
            Wire::Part { .. } => Grant::Part.at(),
            Wire::Land { .. } => Grant::Land.at(),
            Wire::Find(_) => Grant::Find.at(),
            Wire::Trim(_) => Grant::Trim.at(),
            Wire::List(_) => Grant::List.at(),
            Wire::Road(_, _) => Grant::Seek.at(),
            Wire::Name(_) => Grant::Name.at(),
        }
    }

    /// 这一位叫什么（**树上那一段名字**：`/sys/operator/{name}`）。
    pub const fn name(self) -> &'static str {
        match self {
            Grant::Part => "part",
            Grant::Land => "land",
            Grant::Find => "find",
            Grant::Trim => "trim",
            Grant::List => "list",
            Grant::Seek => "seek",
            Grant::Name => "name",
        }
    }

    /// 这一位的记号（**入口 Pie 与会话问话孔同一个**）。
    ///
    /// **两段在这里拼一次**：`"operator-ask-"` ＋ [`Grant::name`] 那一段——面名只有一处，
    /// 记号不抄第二遍字面量。`const fn` 里拼不出 `&str`、也切不出 `&[u8]`，故那两段落进
    /// 一块定长缓冲、按**实际长度**交给 [`Mark::of_bytes`]（同一条 FNV-1a，两侧各算同一个数）。
    pub const fn mark(self) -> Mark {
        const STEM: &[u8] = b"operator-ask-";
        let rest = self.name().as_bytes();
        let mut buf = [0u8; STEM.len() + 4];
        let mut n = 0;
        while n < STEM.len() {
            buf[n] = STEM[n];
            n += 1;
        }
        let mut j = 0;
        while j < rest.len() {
            buf[n] = rest[j];
            n += 1;
            j += 1;
        }
        Mark::of_bytes(&buf, n)
    }
}

/// **认面**：这枚记号是哪一位。
///
/// **答 `None` 不是失败**：今天那条控制面会话的记号（[`ASK_MARK`]）就不是七位之一——它
/// 走的是"会话没说你持哪一柄权"那一档，服务端据此**不判面**（行为与加这一维之前一字不差）。
pub fn grant_of(mark: Mark) -> Option<Grant> {
    let mut i = 0;
    while i < Grant::ALL.len() {
        if Grant::ALL[i].mark().get() == mark.get() {
            return Some(Grant::ALL[i]);
        }
        i += 1;
    }
    None
}

// ── 面不相撞（**编译期**钉住）──────────────────────────────────
//
// 与 `frame.rs` 那两条（`ASK_MARK` ≠ `board-ask` / `ask` / `tip`）同一句正文：这几枚值各是
// 一枚散列，**撞了就是那次装机塌掉**（`ASK_MARK` 的照实记里那次实测 0/3 就是这么来的）。
// 比的是 `.get()` 那个裸值：`Mark` 的 `PartialEq` 不是 `const`，而 `get` 是 `const fn`。
const _: () = assert!(Grant::ALL.len() == 7);
const _: () = {
    let all = Grant::ALL;
    let mut i = 0;
    while i < all.len() {
        // 位次必须逐位对齐（`ALL` 就是位次表）。
        assert!(all[i].at() == (i + 1) as u8);
        // 记号两两不相撞。
        let mut j = i + 1;
        while j < all.len() {
            assert!(all[i].mark().get() != all[j].mark().get());
            j += 1;
        }
        // 与另外三枚（控制面那一枚 / 板那一枚 / 提示那一枚）不相撞。
        assert!(all[i].mark().get() != ASK_MARK.get());
        assert!(all[i].mark().get() != TIP_MARK.get());
        assert!(all[i].mark().get() != Mark::of("board-ask").get());
        i += 1;
    }
};
