// 站点表（site）——挂起任务的唯一容器：唤醒源（`WakeKey`）→ 信标 + 等待者队列。
//
// 分片版（每片一把 L3 锁 + 一个 HashMap），站点寿命与信标操作（`take_beacon` /
// `prune`）都收在本模块。跨到 `messenger` 一级的条目用 `pub(in super::super)`
// （= `messenger`）——`doom.rs` 与 `messenger::rip` 要的那几个，刚好够。

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;

use env::HoleDir;
use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::unit::task::Task;

use super::holder::Ticket;

// ── 类型 ──

/// 唤醒源：谁会把等待者叫醒。三个命名空间各占一个变体，键即身份。
///
/// **没有位打包**：`Space` 的两个字段各自完整，不再把 asid 挤进高 16 位、用户键
/// 截到低 48 位。旧 `WaitKey::compose` 的单射性靠掩码保证，还因此逼出一个
/// `#[inline(never)]` 的 mask helper 去躲 size 优化下的错联（§13.10 A）——枚举下
/// 这两样都不需要：没有 mask，就没有 mask 错联。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WakeKey {
    /// 调用方命名空间里的裸整数（`RoomCall::Wait` / `Wake`）：空间身份 + 槽位。
    Space { space: usize, slot: usize },
    /// 资源就绪（`MailCall::Wait`；hole 的 push / pull / seal 投信）。
    ///
    /// `hole` 用裸整数而非 `mail::HoleId`：依赖方向必须保持 mail → room 单向，
    /// 引 `HoleId` 就成了环。
    Hole { hole: usize, dir: HoleDir },
    /// 目标任务回收（`UnitCall::Join`）。
    Task { id: usize },
    /// 无人投信——只有期限会响（`RoomCall::Park`）。
    ///
    /// 键就是那个睡眠者本人：park 没有信号源，能唤醒它的只有它自己那次到点登记。
    Alarm { task: usize },
}

impl WakeKey {
    /// 折成 64 位——**只供分片**，不承载语义（相等性仍由 `Eq` 判定）。
    pub(super) fn fold(self) -> u64 {
        match self {
            WakeKey::Space { space, slot } => {
                (space as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ slot as u64
            }
            WakeKey::Hole { hole, dir } => {
                (hole as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ dir as u64
            }
            WakeKey::Task { id } => (id as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93),
            WakeKey::Alarm { task } => (task as u64).wrapping_mul(0xA24B_AED4_963E_E407),
        }
    }

    /// 分门别类用的**分类标签**（审计观测面用；语义与变体一一对应，不是折叠值）。
    #[cfg(feature = "audit")]
    pub(in super::super) const fn kind(self) -> WakeKind {
        match self {
            WakeKey::Space { .. } => WakeKind::Space,
            WakeKey::Hole { .. } => WakeKind::Hole,
            WakeKey::Task { .. } => WakeKind::Task,
            WakeKey::Alarm { .. } => WakeKind::Alarm,
        }
    }
}

/// 唤醒源的四类命名空间——只管「这站点在等什么」，供审计计数分列。
///
/// 判别式即 [`WakeKind::ALL`] 的下标（`as usize`），故四类合计可直接按数组求和。
/// 只在 audit 档（`SiteStats::kinds`）有消费者——非 audit 构建整型 cfg out。
#[cfg(feature = "audit")]
#[derive(Clone, Copy, Debug)]
pub(in super::super) enum WakeKind {
    Space,
    Hole,
    Task,
    Alarm,
}

#[cfg(feature = "audit")]
impl WakeKind {
    /// 全部四类，下标 = 判别式：分列数组与打印顺序都由它一处定。
    pub(in super::super) const ALL: [WakeKind; 4] = [
        WakeKind::Space,
        WakeKind::Hole,
        WakeKind::Task,
        WakeKind::Alarm,
    ];

    /// 分列用的名字（打印与排序的唯一出处）。
    pub(in super::super) const fn name(self) -> &'static str {
        match self {
            WakeKind::Space => "space",
            WakeKind::Hole => "hole",
            WakeKind::Task => "task",
            WakeKind::Alarm => "alarm",
        }
    }
}

/// 一个唤醒源的等待位：遗留信号（信标）+ 等待者队列。
///
/// 三种唤醒源共用本类型（旧版 `WaitSite` / `JoinSite` 字段逐个相同——各自一份是
/// 键的 Rust 类型不同逼出来的）。
pub(in super::super) struct Site {
    /// 遗留信号（信标）：wake 无等待者 → 置位；wait 见位 → 消费即回（防漏唤醒）。
    pub(in super::super) pend: bool,
    /// 等待者（FIFO）；每项携带到点句柄（无期限 = None）。
    pub(in super::super) waiters: VecDeque<Waiter>,
}

/// 等待者：站点队列里的一项。票号即「哪一次挂起」——同一任务先后等同一个键时，
/// 靠它区分，故陈旧的到点登记不可能偷走后来的那次等待。
pub(in super::super) struct Waiter {
    pub(in super::super) task: Arc<Task>,
    pub(in super::super) ticket: Ticket,
}

// ── 簿记表（全部 L3） ──

/// 事件等待表的分片数。每片 = 一把 L3 锁 + 一个 HashMap；wait/wake/drain
/// 按 [`site_shard`] 纯函数路由到同片，跨片互不阻塞——把单点串行竞争降到
/// 1/SITE_SHARDS（典型 16）。分片数取 2 的幂：位与替代 mod。
pub(in super::super) const SITE_SHARDS: usize = 16;
const SITE_SHARDS_MASK: usize = SITE_SHARDS - 1;

/// 唤醒源 → 分片（pure function，所有路径一致：wait / wake / 投信 / 到期都经此）。
/// splitmix64 折叠 64→32 后按位与 SHARDS 掩码——高位低位的熵都被采样。
#[inline]
fn site_shard(key: WakeKey) -> usize {
    let h = key.fold().wrapping_mul(0x9E3779B97F4A7C15);
    ((h >> 32) ^ h) as usize & SITE_SHARDS_MASK
}

/// 站点表（Level::L3，绝不 3→3 嵌套）。**分片版**：每片一把
/// L3 锁 + HashMap，单一线性化点缩小到一片——wait / wake 跨片并行。
///
/// **一张表装三种唤醒源**：它们的等待者是同一种东西（任务 + 到点句柄），唤醒
/// 也是同一件事（摘出 → 放回就绪）。旧版把「等目标回收」单独放进 `joins`，理由
/// 只是键的 Rust 类型不同（`usize` vs `WaitKey`）——键成枚举之后，那个理由没了。
///
/// 锁纪律：仍是 L3、可与 timer 锁共存但**绝不 3→3 嵌套**（rip 路径循环逐片清，
/// 禁持跨片锁）。同 key 的所有 waiters 必落在同一分片（`site_shard` 纯函数保证），
/// 唤醒不必跨片扫描。
pub(in super::super) fn shard_at(shard: usize) -> &'static SpinLock<HashMap<WakeKey, Site>> {
    static SHARDS: OnceLock<Box<[SpinLock<HashMap<WakeKey, Site>>]>> = OnceLock::new();
    let arr: &'static [SpinLock<HashMap<WakeKey, Site>>] = SHARDS.get_or_init(|| {
        let mut v: Vec<SpinLock<HashMap<WakeKey, Site>>> = Vec::with_capacity(SITE_SHARDS);
        for _ in 0..SITE_SHARDS {
            v.push(SpinLock::new_level(Level::L3, HashMap::new()));
        }
        v.into_boxed_slice()
    });
    &arr[shard]
}

/// 本唤醒源的站点表分片。
pub(in super::super) fn sites(key: WakeKey) -> &'static SpinLock<HashMap<WakeKey, Site>> {
    shard_at(site_shard(key))
}

/// 信标先探：消费本键上的遗留信号。**缺键即无信标**——不 `or_insert`：空的、
/// 无信标的站点没有语义，不该被「先探」凭空造出来。
pub(super) fn take_beacon(key: WakeKey) -> bool {
    let mut sites = sites(key).lock();
    match sites.get_mut(&key) {
        Some(site) if site.pend => {
            site.pend = false;
            true
        }
        _ => false,
    }
}

/// 站点存在的判据：**队列非空 ∨ 有信标**。出队之后若不成立即删——空壳站点没有
/// 语义，留着就是 A2 那条「站点永不回收」的老毛病（`park` 每次睡眠都会留一个）。
/// 前置：已持有该分片的锁。
///
/// 因此站点有三种形态，审计计数（`messenger::probe`）按它们分列：
///   - **活**（`waiters` 非空）：有任务挂在这里；
///   - **墓碑**（`pend == true`，队列空）：`wipe` 留下的「此键已退役」结论，语义仍
///     有效（后来的等待者要当场拿到它），**本函数依判据保留**——故它不算违规；
///   - **孤儿**（队列空 **且** 无信标）：没有任何语义，正是本函数该删的那一类。
/// 判别式「孤儿 == 0」才是对 `prune` 的直接断言（墓碑会稀释总数，见 §9.3 实测）。
pub(in super::super) fn prune(sites: &mut HashMap<WakeKey, Site>, key: WakeKey) {
    if let Some(site) = sites.get(&key)
        && site.waiters.is_empty()
        && !site.pend
    {
        sites.remove(&key);
    }
}

// ── 内部辅助 ──

impl Site {
    pub(super) fn new() -> Self {
        Self {
            pend: false,
            waiters: VecDeque::new(),
        }
    }
}
