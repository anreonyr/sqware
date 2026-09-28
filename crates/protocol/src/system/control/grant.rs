//! control::grant —— **面那一维**：一枚 `Grant` = 一条权柄边界。
//!
//! ```text
//!   State 问阶段   Mint 建一条   Start 放行＋等就绪   Stop 下令收掉     ← 四条原语，一原语一面
//!   位次 1..=4：State / Mint / Start / Stop          → /sys/control/{state,mint,start,stop}
//! ```
//!
//! # 这一族**不折面**（用户两次裁定）
//!
//! 另外三家的面都少于原语条数（operator 七位里有几条并成一面、principal / coalition 各折成
//! `Ask` / `Set`）。这一族**一原语一面**：先驳回了"折成 `Ask` / `Set`"，再驳回了"`Start` 与
//! `Stop` 合一面"。故**名字就是原语自己的名字**（与 operator 那七位同形：
//! `/sys/operator/{part,land,find,trim,list,seek,name}`），不折成角色词。
//!
//! # 这一族为什么不是"数持有者"数出来的
//!
//! 另外三家都是先数生产里的持有者再定面数。这一族数出来的只有**一位**，而且是一位**测具**
//! （`harness/src/probe_control.rs`：它握着那一枚入口，一辈子只叫 `State` 一条）——`Mint` /
//! `Start` / `Stop` 那三条它**做得出、一次没叫过**。
//!
//! 故四格是按**裁定**切的，不是数出来的：一个原语一条权柄边界。**照实记（三面今天零持有者，
//! 照样开）**：`Mint` / `Start` / `Stop` 三面在生产里零持有者（装配者叫的是**进程内**的
//! `Control`，不经这一面）——与盟册 `Set` 那一面同一条照实记（"今天生产零持有者，照样开"）。
//! 格数不是"将来可能有客人"推出来的，是**用户裁的**。
//!
//! # 哪一面带规矩（"动"落在这里）
//!
//! - [`Grant::State`] **公开**：`Permit::Unset`——只读"这一条此刻在哪个阶段"，谁问都一样；
//! - [`Grant::Mint`] / [`Grant::Start`] / [`Grant::Stop`] **各带一句规矩**：落格时带
//!   `Permit::Opener(那一格自己的号)`——"**许给开着这一格的那位**"，而四枚入口都是**装配者主线程**
//!   铸的 ⇒ 这句话指的就是它（见 `programs/src/system/mod.rs::mount_control`）。
//!
//! **拆面把"动"的代价抹掉了**：没有这一刀之前，给 `/sys/control` 带规矩只有一个后果——那台测具
//! "另一个域**取得到** control 门牌"的正证当场翻面。四面各归各的之后：问面照旧公开（那台探针照旧
//! 找得到、问得着），规矩只落在三条**改机器**的面上（它取那三格被拒——那正是读数）。
//!
//! **照实记（记号那一格撞了）**：这一族**有会话**——开会话那一枚问话孔是
//! [`ASK_MARK`](crate::system::control::ASK_MARK)（`"control-ask"`，见 [`super::frame`]）⇒ 四面的
//! **入口**记号不能也叫 `control-*`，故词根取 `"control-entry-"`：**入口是入口、会话是会话**，
//! 两个名字都留着（`faces!` 的 `distinct` 那一格在编译期就拦住撞名）。

use env::Mark;

use super::frame::{ASK_MARK, BACK, Wire};

crate::faces! {
    /// **一条权柄边界**：一枚 = 一面。四位，位次 1..=4。
    pub enum Grant {
        /// **问**：这一条此刻处于哪个阶段（`State`）——只读，不动机器一根手指。
        State => "state",
        /// **造**：按名字起一条（建域 ＋ 产代表线程，恒产未放行）。
        Mint => "mint",
        /// **放行**：放行 ＋ 等就绪。
        Start => "start",
        /// **收**：下令收掉（下令即回）。
        Stop => "stop",
    }
    stem: "control-entry-",
    name_max: 5,
    wire_ty: Wire,
    wire: {
        // 四条线上码逐条说它落哪一面——**一码一面，不并 `|`，也不并面**（用户裁定）。
        Wire::State(_) => State,
        Wire::Mint(_) => Mint,
        Wire::Start(_) => Start,
        Wire::Stop(_) => Stop,
    }
    distinct: [
        // 本族自己那两枚（会话的问话孔 ＋ 回信孔）也在这里：它们与四面的**入口**记号必须不同。
        ASK_MARK,
        BACK,
        // 别族的记号按**字面量**给（不跨族 `use`）：入口通用那一枚、提示那一枚、板那一枚、
        // 树那七位里的第一位、名册那两面、盟册那两面。
        Mark::of("entry"),
        Mark::of("tip"),
        Mark::of("board-ask"),
        Mark::of("operator-ask-part"),
        Mark::of("principal-ask"),
        Mark::of("principal-set"),
        Mark::of("coalition-ask"),
        Mark::of("coalition-set"),
    ],
}
