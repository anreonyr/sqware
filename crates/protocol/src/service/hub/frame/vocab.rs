//! :frame 的词汇那一半：三面码（BOND/LIST/CLAIM）· 上限（LIST_MAX）·
//! 记号（`BACK_MARK`/`ALIVE_MARK`）· 设备路与三枚键名（`DEV_ROAD`/`BOOT`/`DTB`/`IRQ`）· 失败词汇（`Fail`）。

use crate::common::path::Path;
use crate::wire::OK;

/// 报名：许我驱这一类。
pub const BOND: u8 = 1;

/// 列册：这一类里现在有哪几台、哪几台有主。
pub const LIST: u8 = 2;

/// 认领：这台归我。
pub const CLAIM: u8 = 3;

/// 一窗最多几条（取窗宽度）。**它是个旋钮，不是契约**——把 `Buf` 撑大就调它。
pub const LIST_MAX: usize = 4;

/// 回信孔那一枚上的记号。**三条问共用**：回信孔是每一趟自带的，与面无关。
pub const BACK_MARK: env::Mark = env::Mark::of("hub-back");

/// **报活孔**那一枚上的记号：主人（认领那一台的那位）铸一枚、**交一份给 hub**、此后一直开着。
/// hub 扫账时按它问"主人还在不在"（mail::reserve——与线路由者那条探活同一手）。内核那一问
/// （UnitCall::Join）只许**同队或父域**，而 hub 与驱动是**兄弟** ⇒ 主人那一枚只能由主人
/// 自己交过来。
pub const ALIVE_MARK: env::Mark = env::Mark::of("hub-alive");

/// **设备那一轴在树上的路**：`/dev`（`/dev/<类>/<名>` 的头一段）。
pub const DEV_ROAD: &Path = Path::new("dev");

/// **boot 那一类**：引导期那两件不按 `compatible` 认的东西（设备树本体 / 门铃）落在它底下
/// （`/dev/boot/{dtb,irq}`）——它们与设备同一条账（认领读法一模一样），只是"类"不是树里给的。
pub const BOOT: &str = "boot";

/// boot 那一类底下那两格的名字：**设备树本体**（hub 自己也要用它读名 / 类 / 线，
/// 但它同时是**树那一侧的客户**（`router` 要读 `riscv,ndev` 与 `interrupts-extended`））。
pub const DTB: &str = "dtb";

/// boot 那一类底下那两格的名字：**门铃**（中断那枚空载荷信号）。
pub const IRQ: &str = "irq";

/// 四格 ＋ 一格"读不懂"。**前四格对应四个不同的下一步**；Fail::Bad 是本端那一格。
#[derive(Clone, Copy, PartialEq, Eq, Debug, crate::WireCodes)]
pub enum Fail {
    /// 没这件 / 这一类不在册。
    #[code(1)]
    Unknown,
    /// 有活着的主人。
    #[code(2)]
    Taken,
    /// 授不出。
    #[code(3)]
    Denied,
    /// 那一枚孔用不动（**本端判的**：这一枚的资源没了 / 权限不够 / 已交出去）。
    #[code(4)]
    Dead,
    #[code(5)]
    Bad,
}
