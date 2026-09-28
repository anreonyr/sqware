//! principal::grant —— **面那一维**：一枚 `Grant` = 一条权柄边界。
//!
//! ```text
//!   Bind 定   Resolve 问   Derive 生   Adopt 领   Waive 弃   Sire 父   Heir 支   ← 七条原语
//!   Ask 问面：Resolve / Sire / Heir          Set 定面：Bind / Derive / Adopt / Waive
//! ```
//!
//! # 两面是**数出来**的，不是照条数切的
//!
//! 七条原语若照条数拆七格，其中三格**今天没有任何生产持有者**（`Adopt` / `Waive` / `Sire`
//! 只被测具选过）——那正是"报法字段"：造出来没人选。生产里的持有人只数得出三个：
//!
//! | 持有者 | 它实际叫哪几条 |
//! |---|---|
//! | 持树者的名册门（`operator/door.rs` 的 `Court`） | `Resolve` ＋ `Heir` |
//! | 盟册服务自己（`coalition/server.rs`） | `Resolve` |
//! | 装配者那一侧（`principal/bridge.rs` 的 `Roster`） | `Derive` ＋ `Bind` |
//!
//! ⇒ **三条"问"的 ＋ 四条"定"的**，一面一条。而"问的一条与定的那条同一面"是有后果的，且量得
//! 出来：**持树者手里那枚门牌做得出 `Adopt`**（把一条号领到自己底下——`probe-rule` 量的那一格：
//! `adopt(q)` 之后 `Trunk(p)` 立刻答 `Denied`），而它一辈子只用两条**读**的。这与 operator 拆
//! 七格是同一条理由（`find` 会交出能力 ⇒ 不与只读那几条合成一位）。
//!
//! # 面**就是那一枚门牌**（这一族与 operator 不同的一格）
//!
//! operator 那一族有会话：面在**会话入口的记号**上（`ask()`），服务端从它自己表里那枚问话孔
//! 读回来。principal **没有会话**——门牌自己就是那条路，而今天所有人往**同一枚**孔推帧，
//! 服务端因此**分不出面**。故这一族的面只能是**两枚门牌、两只孔**：服务端从**它自己表里哪一枚
//! 孔**收到，就是哪一面。那一枚门牌自己的记号仍然要，但只给**树 / 装配者查表**用——它们按
//! `(开者, 记号)` 两格在各自表里认出那一枚（`establish::find` / `operator::claim::find_face`）。
//!
//! **不上帧**：本文件一个字节都不进报文——线格式、动作码、失败域**一处未动**。
//!
//! # 位次与记号
//!
//! [`Grant::ALL`] 的**次序就是位次 1..=2**（第 i 位 = `ALL[i-1]`）——位次与枚举变体不各说各的。
//! 记号 = `"principal-" ＋ 面名`：它同时是**那一枚门牌 Pie 的记号**与**下面那一段入口路的
//! 末段名**（`/sys/principal/{ask,set}`），一个面名一处给。
//!
//! **照实记（两面各带自己的记号，没有一面留 `entry`）**：可以图省事让问面继续用那枚通用的
//! `board::ENTRY_MARK`、只给定面新记号——那样 `find_face(who)` 一个字都不用改。**被否**：
//! "没有记号的那一面是读面"是**位置即语义**（靠"哪一枚是默认的"分派），而这里恰恰是**判面**
//! 那一格；两面各带显式的记号之后，查表那一侧必须**说清自己要哪一面**（`find_face(who, mark)`）。

use env::Mark;

use super::frame::BACK;
use super::frame::Wire;

/// **一条权柄边界**：一枚 = 一面。两位，位次 1..=2。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grant {
    /// **问面**：`Resolve` / `Sire` / `Heir`——只读，不改任何一个字。
    Ask,
    /// **定面**：`Bind` / `Derive` / `Adopt` / `Waive`——改身份或改谱系。
    Set,
}

impl Grant {
    /// 两位，**次序即位次**（第 i 位在 `ALL[i-1]`）。
    pub const ALL: [Grant; 2] = [Grant::Ask, Grant::Set];

    /// **这一位是几**（1..=2）——判面那一句比的就是它。
    ///
    /// 次序由 [`Grant::ALL`] 给，故这里不另写一张表：改枚举次序那条路根本不存在。
    pub const fn at(self) -> u8 {
        let mut i = 0;
        while i < 2 {
            if Grant::ALL[i] as u8 == self as u8 {
                return (i + 1) as u8;
            }
            i += 1;
        }
        // 不可能：每一位都在 `ALL` 里（加变体不补 `ALL` ⇒ 这条循环走到底）。
        panic!("principal: grant not in ALL")
    }

    /// **这一问属于哪一面**——穷尽 `match`：加一条线上动作不补这里 ⇒ 编不过。
    ///
    /// 三条读的落 `Ask`，四条定的落 `Set`；**每一面至少有一条**（下面的编译期断言钉住）。
    pub const fn of_wire(wire: &Wire) -> u8 {
        match wire {
            Wire::Resolve(_) | Wire::Sire(_) | Wire::Heir(_, _) => Grant::Ask.at(),
            Wire::Bind(_, _) | Wire::Derive(_) | Wire::Adopt(_) | Wire::Waive => Grant::Set.at(),
        }
    }

    /// 这一面叫什么（**树上那一段名字**：`/sys/principal/{name}`）。
    pub const fn name(self) -> &'static str {
        match self {
            Grant::Ask => "ask",
            Grant::Set => "set",
        }
    }

    /// 这一面的记号（**那一枚门牌 Pie 的记号**）。
    ///
    /// **两段在这里拼一次**：`"principal-"` ＋ [`Grant::name`] 那一段——面名只有一处，记号不抄
    /// 第二遍字面量。`const fn` 里拼不出 `&str`、也切不出 `&[u8]`，故那两段落进一块定长缓冲、
    /// 按**实际长度**交给 [`Mark::of_bytes`]（同一条 FNV-1a，两侧各算同一个数）。
    pub const fn mark(self) -> Mark {
        const STEM: &[u8] = b"principal-";
        let rest = self.name().as_bytes();
        let mut buf = [0u8; STEM.len() + 3];
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

/// **认面**：这枚记号是哪一面。
///
/// **答 `None` 不是"失败"**，是"这一枚不是本族的面"——通往名册的孔有两种来路（本族那两面，
/// 与别的域借 `entry` 那枚通用记号铸的自家入口），而只有前两种该被本协议的面判住。
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

// ── 面不相撞 ＋ 位次对齐（**编译期**钉住）────────────────────────
//
// 与 `operator/grant.rs` 那一组同一句正文：记号各是一枚散列，**撞了就是那次装机塌掉**。
// 比的是 `.get()` 那个裸值：`Mark` 的 `PartialEq` 不是 `const`，而 `get` 是 `const fn`。
// 别的族的记号按**字面量**比（不跨族 `use`）——同 operator 那一处的做法。
const _: () = assert!(Grant::ALL.len() == 2);
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
        // 与本族那两枚（回信孔 / 入口通用记号）＋ 另外几族的记号不相撞。
        assert!(all[i].mark().get() != BACK.get());
        assert!(all[i].mark().get() != Mark::of("entry").get());
        assert!(all[i].mark().get() != Mark::of("tip").get());
        assert!(all[i].mark().get() != Mark::of("board-ask").get());
        assert!(all[i].mark().get() != Mark::of("control-ask").get());
        assert!(all[i].mark().get() != Mark::of("operator-ask-part").get());
        i += 1;
    }
};
