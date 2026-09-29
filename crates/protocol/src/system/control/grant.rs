//! control::grant —— **面那一维**：一枚 `Grant` = 一条权柄边界。
//!
//! ```text
//!   State 问阶段   Mint 建一条   Start 放行＋等就绪   Stop 下令收掉     ← 四条原语，一原语一面
//!   位次 1..=4：State / Mint / Start / Stop          → /svc/sys/control/{state,mint,start,stop}
//! ```
//!
//! # 这一族**不折面**（用户两次裁定）
//!
//! 另外三家的面都少于原语条数（operator 七位里有几条并成一面、principal / coalition 各折成
//! `Ask` / `Set`）。这一族**一原语一面**：先驳回了"折成 `Ask` / `Set`"，再驳回了"`Start` 与
//! `Stop` 合一面"。故**名字就是原语自己的名字**（与 operator 那七位同形：
//! `/svc/sys/operator/{part,land,find,trim,list,seek,name}`），不折成角色词。
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
//!   [`Rule::Root`](crate::system::operator::Rule::Root)——**"许给根"**（`Permit::Trunk(ROOT)`）。
//!
//! **照实记（这一句原先写的是"许给开着这一格的那位"——真机一量是假的）**：上一版计划写的是
//! `Permit::Opener(那一格自己的号)`，理由是"四枚入口都是装配者主线程铸的 ⇒ 这句话指的就是它"。
//! **一量就翻**：`operator: opens sealed n=33`——**根那批孔是引导期的设备门闩，`opened_by` 的
//! `owner` 格是 0** ⇒ 那一格的"开者"根本答不出来 ⇒ 挂在那格上的规矩**永久判不了**
//! （`probe-control` 读到的正是 `Err(Unjudged)`）。故规矩换成"许给根"。
//!
//! **诚实的一笔**：根**没有名册身份**（`bind: false`）⇒ 这一条今天对**所有人**都判拒，**包括
//! 装配者自己**；而装配者根本不走这条路（它叫的是**进程内**的 `Control`）⇒ 效果就是"这三面谁也
//! 取不回"。这正是 `control/mod.rs` 头注里那句"**要收，收的是那一格的 `Permit`**"——那个口子
//! （"任何已绑身份的域都能 `mint` / `start` / `stop` 装配表里任意一台"）今天收上了。
//!
//! **设置那一句落在哪一行**：`programs/src/system/mod.rs::Assembly::mount_control`。**本文件只说
//! "哪一面带"，不说"怎么带"**——那一句 `match` 与那枚 `Rule` 住正文那一格（回炉那一刀把这句话
//! 的两份抄写收成了一份，收在这一格：面是**本协议**的词）。
//!
//! **拆面把"动"的代价抹掉了**：没有这一刀之前，给 `/svc/sys/control` 带规矩只有一个后果——那台测具
//! "另一个域**取得到** control 门牌"的正证当场翻面。四面各归各的之后：问面照旧公开（那台探针照旧
//! 找得到、问得着），规矩只落在三条**改机器**的面上（它取那三格被拒——那正是读数）。
//!
//! **照实记（记号那一格撞了）**：这一族**有会话**——开会话那一枚问话孔是
//! [`ASK_MARK`](crate::system::control::ASK_MARK)（`"control-ask"`，见 [`super::frame`]）⇒ 四面的
//! **入口**记号不能也叫 `control-*`，故词根取 `"control-entry-"`：**入口是入口、会话是会话**，
//! 两个名字都留着（那一撞由 [`crate::system`] 的全族总表在编译期拦住——照实记见
//! [`crate::faces!`]）。

use super::frame::Wire;

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
}
