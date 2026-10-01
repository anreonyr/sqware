//! :frame 的词汇那一半：身份号（PrincipalId）· 失败词汇（Fail）· 动作码与
//! 状态码 · 记号与那一段路（`BACK`/`DIR`/`NAME`）。

use env::Mark;

use crate::common::path::Path;
use crate::wire::OK; // `WireCodes` 派生的两向读法要用它（本文件是枚举的家）

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PrincipalId(usize);

impl PrincipalId {
    /// 根：Server 启动时自带的那一枚，**唯一没有父的节点**。
    pub const ROOT: PrincipalId = PrincipalId(0);

    pub const fn new(raw: usize) -> PrincipalId {
        PrincipalId(raw)
    }

    /// 裸号。
    pub const fn get(self) -> usize {
        self.0
    }
}

/// 失败域：三格，每格一个**不同的下一步**。
/// **`Resolve` 与三条谱系读没有失败域**——读是公开的（答案不是秘密，Principal 不授予任何
#[derive(Clone, Copy, PartialEq, Eq, Debug, crate::WireCodes)]
#[wire(also(BAD = 4))]
pub enum Fail {
    /// 你不是那一个：不是写名册的那一枚（`Bind`/`Unbind`）、不是"当前正好代表 `p`"的那一枚
    /// （`derive`）、或目标不在**你自己那一支**里（`adopt`）。调用方要改的是：**该请谁来做**
    /// 或**换一个目标**。
    #[code(1)]
    Denied,
    /// 这条 PrincipalId 不在树里，或这个 TID 没绑过。调用方要改的是：**我手里这个号是假的**。
    #[code(2)]
    Unknown,
    /// `try_reserve` 备不下。调用方要改的是：**晚点再来**。
    #[code(3)]
    Full,
}

/// 七条线上动作——**与核心那七条同名**（核心另有 `unbind` / `clan` 两条**不上线**，见正文
/// 那张表）：线上与模型是同一件事的两层，不该各起一套词。
pub const BIND: u8 = 1;

pub const RESOLVE: u8 = 2;

pub const DERIVE: u8 = 3;

pub const SIRE: u8 = 4;

pub const HEIR: u8 = 5;

/// 转换 · 领：`a` = 目标号（发送者由内核盖章，报文里没有"我是谁"那一格）。
pub const ADOPT: u8 = 6;

/// 转换 · 弃：两格都空——它只认"发送者是谁"。
pub const WAIVE: u8 = 7;

pub const DROP: u8 = 8;

// 长度、编 / 解、答话那几手**本体在 crate::frame**——principal 与 coalition 同形，故只有
// 都不用改）。**本族自己的**是下面那些：码、`reply_present`、失败表、记号。

/// 同一张表里就分不出这一枚是哪一面的。
pub const BACK: Mark = Mark::of("principal-back");

/// **本族那块窗格在树上的路**：`/svc/sys/principal`（头两段是四族共用的
/// crate::common::svc::DIR，末段是本族自己的名字 NAME）。
pub const DIR: &Path = Path::new("svc/sys/principal");

/// 本服务在树上的那一段名字：`/svc/sys/principal`——**它不是一格**（
/// 两枚门牌是它底下那两格 `/svc/sys/principal/{ask,set}`，末段名由
/// Grant::name 给）。
pub const NAME: &str = "principal";
