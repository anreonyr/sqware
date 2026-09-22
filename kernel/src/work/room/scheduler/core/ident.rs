// 本核身份槽（core::ident）— 身份两态、带标签的一格字与唯一读法 `ident()`。
//
// 载荷两态：Live = 本核**在跑**任务（trap 可信）；Last = 末次身份记录（任务号 /
// 域号；trap **不可信**且类型上不可读）。标签位与载荷在**同一个原子字**里——载荷
// 类型自描述，读侧无需第二读点（两个写点无读撕裂窗口）。
//
// 计数协议：槽**只在 Live 上**持有一份 Arc 强引用。写只有两个方法——[`Badge::seat`]
// （装 Live）/ [`Badge::shed`]（换 Last）；旧载荷的归还只有一处实现（`Badge::reclaim`），
// 而它只剩 Live 一臂有物可还。读 = `Badge::read`（Acquire + increment_strong_count）。
// 同 hart 单写单读 ⇒ 无锁；跨核读是 UB（槽私有，只经 [`ident`] 触及，
// 而它读的必是本核的槽）。
//
// **照实记（清槽原语为什么不在）**：这里曾有 `Badge::clear`，它唯一可观察的作用是在
// 关机基线审计前放掉槽里那份 Last 计数，免得记成块差集里的假泄漏（当年实证 4 hart = 4
// 笔；注释里那个 48 B 是 `LastIdent` 还内联名字时的读数——名字那一刀之后这个类型只剩
// 两枚号，账面应为 32 B。**读数有寿命**）。末次那一态现在压进那一格字、不持计数 ⇒
// 无物可还，`clear` 随之退出；连带消失的是 `rip` 里"遍历别核槽清空"那条**跨核写**，
// "同 hart 单写单读"从此无例外。**空槽也只剩开机一种来路**：`seat` 把空槽变 Live、
// `shed` 把 Live 变 Last，没有回空的路。

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

use env::{TaskId, TeamId};

use crate::memory::manager::addr::PhysAddr;
use crate::work::unit::task::TaskIdent;

use super::hart::frame_pa;
use super::table::{SCHEDULERS, current};

/// 末次身份记录：降级时从 [`TaskIdent`] 抄两枚号，**不含 team/space/trap**——
/// 团队 Arc 借此归零即回收整个地址空间。
///
/// 它同时是那一格字的**编码**（[`pack`](LastIdent::pack) /
/// [`unpack`](LastIdent::unpack)）：两枚号与标签位拼成一个 `usize`，故末次那一态
/// **零分配、无计数、无需归还**。
///
/// **照实记**：这个类型曾是 `Arc<LastIdent>`，还内联过一份角色名与域名字；名字那一刀
/// 只剩两枚号，这一刀连 `Arc` 也去掉（每次泊车的一次堆分配、以及它失败即整机 halt 的
/// 那枚 panic 一起消失）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LastIdent {
    pub(crate) task: TaskId,
    pub(crate) team: TeamId,
}

impl LastIdent {
    /// 标签位：`Arc` 指针 8 对齐 ⇒ bit0 恒 0，故这一位可当载荷类型标签。
    const TAG: usize = 1;
    /// 域号占 bit1..=bit31。
    const TEAM_BITS: u32 = 31;
    /// 任务号自域号之后起（= 1 + `TEAM_BITS`——**不重叠由这条算术保证**，不靠注释）。
    const TASK_SHIFT: u32 = 1 + Self::TEAM_BITS;
    const TEAM_MASK: usize = (1 << Self::TEAM_BITS) - 1;

    /// 这一格字是不是末次载荷——标签位的**唯一**判据（[`Payload::of`] 只经它看那一位）。
    const fn tagged(raw: usize) -> bool {
        raw & Self::TAG != 0
    }

    /// 编码：`TAG | team << 1 | task << TASK_SHIFT`。
    ///
    /// **照实记**：两枚号的界（`team < 2^31`、`task < 2^32`）是**资源**给的，不是设计
    /// 挑的——一枚任务至少占栈 + 帧两帧（2^32 枚 ≈ 32 TiB），一枚域至少占一张根页表帧
    /// （2^31 枚 ≈ 8 TiB），都超出物理地址空间可达。故这里不检查、调用处也不设断言。
    const fn pack(self) -> usize {
        Self::TAG | (self.team.get() << 1) | (self.task.get() << Self::TASK_SHIFT)
    }

    /// 解码。前置：`tagged(raw)`——本函数不自己问（问的人先问）。
    const fn unpack(raw: usize) -> LastIdent {
        LastIdent {
            task: TaskId::new(raw >> Self::TASK_SHIFT),
            team: TeamId::new((raw >> 1) & Self::TEAM_MASK),
        }
    }
}

/// 那一格字的**三种含义**：空槽 / 在跑任务的 `Arc` 指针 / 末次两枚号。
///
/// 存在的理由只有一条：[`Badge::read`] 与 [`Badge::reclaim`] 都得先知道这一格是什么，
/// 而"怎么判"只写在这里一处。
#[derive(Clone, Copy)]
enum Payload {
    Empty,
    Live(*const TaskIdent),
    Last(LastIdent),
}

impl Payload {
    /// 全函数：0 = 空槽；带标签 = 末次；否则 = 指向 [`TaskIdent`] 的 `Arc` 指针。
    fn of(raw: usize) -> Payload {
        if raw == 0 {
            Payload::Empty
        } else if LastIdent::tagged(raw) {
            Payload::Last(LastIdent::unpack(raw))
        } else {
            Payload::Live(raw as *const TaskIdent)
        }
    }
}

/// 身份槽载荷：Live = 本核**在跑**任务（trap 可信）；Last = 末次身份记录
/// （任务号 + 域号；trap **不可信**且类型上不可读）。trap 只经 Live 轴暴露——
/// 悬垂帧读取在类型层不可表达。
pub enum Identity {
    Live(Arc<TaskIdent>),
    Last(LastIdent),
}

/// 本核身份槽（工牌）：谁在本核上跑。**唯一写点 = 两个方法**，回收只有一个实现。
///
/// 字段私有：读只经 [`ident`]，写只经 `seat` / `shed`（调用点都在 `core::hart`），
/// 于是「带标签的一格字 + arc 计数交换」这套协议只活在本文件里。
pub(super) struct Badge {
    raw: AtomicUsize,
}

impl Badge {
    /// 空槽（未装牌）。
    pub(super) fn new() -> Badge {
        Badge {
            raw: AtomicUsize::new(0),
        }
    }

    /// 装牌：写入 Live 载荷（本核在跑任务）。`Arc::into_raw` 交出克隆的计数给槽持有。
    pub(super) fn seat(&self, ident: &Arc<TaskIdent>) {
        let prev = self
            .raw
            .swap(Arc::into_raw(ident.clone()) as usize, Ordering::AcqRel);
        Self::reclaim(Payload::of(prev));
    }

    /// 换牌：Live → Last（本核在跑任务离核且不接续装槽：reap / park 无后继）。
    /// Last 只留两枚号（任务号 + 域号），**不持有团队/空间**——团队 Arc 借此归零即
    /// 回收，地址空间不再被 idle 核钉住；而它压进那一格字 ⇒ **零分配、恒成功**
    /// （旧形状要在这一步 `Arc::new`，失败即 `handle_alloc_error` → 整机 halt：
    /// 泊车这条常走路径上曾挂着一枚 panic）。
    pub(super) fn shed(&self, ident: &TaskIdent) {
        let last = LastIdent {
            task: ident.id,
            team: ident.team.id,
        };
        let word = last.pack();
        // 两枚号的解码值只在崩溃报告里被读（正常跑不打印），故往返在这条常走路径上
        // 当场自证：debug 档每次泊车验一次（release 编译掉，零成本）。
        debug_assert_eq!(LastIdent::unpack(word), last, "末次编码往返");
        let prev = Payload::of(self.raw.swap(word, Ordering::AcqRel));
        // 降级只在拥有在跑任务时发生——旧载荷必为 Live。
        debug_assert!(matches!(prev, Payload::Live(_)), "shed 旧载荷非 Live");
        Self::reclaim(prev);
    }

    /// 读：None = 未装槽。载荷类型按标签位判定（标签与载荷原子同行，无撕裂）。
    fn read(&self) -> Option<Identity> {
        match Payload::of(self.raw.load(Ordering::Acquire)) {
            Payload::Empty => None,
            Payload::Last(l) => Some(Identity::Last(l)),
            Payload::Live(p) => {
                // SAFETY: 未标签 = TaskIdent 载荷；槽持有者对 p 保有一份计数（Acquire
                // 与存入侧 AcqRel 配对，Arc 数据已发布）。同 hart 程序序下本核读时无
                // 并发 swap——increment 后再 from_raw 克隆，归还时计数一致。
                unsafe {
                    Arc::increment_strong_count(p);
                    Some(Identity::Live(Arc::from_raw(p)))
                }
            }
        }
    }

    /// 归还旧载荷持有的那份计数（两个写点唯一的回收实现）：**只有 Live 有物可还**。
    fn reclaim(prev: Payload) {
        if let Payload::Live(p) = prev {
            // SAFETY: 未标签 = TaskIdent 载荷；swap 取走后槽对其不再持有，此处
            // from_raw 收回该份计数并释放（同 hart 程序序，无并发的本槽读写）。
            unsafe {
                drop(Arc::from_raw(p));
            }
        }
    }
}

impl Identity {
    /// 任务号（两态都答）。
    pub fn task_id(&self) -> usize {
        match self {
            Identity::Live(t) => t.id.get(),
            Identity::Last(l) => l.task.get(),
        }
    }

    /// 域号（两态都答；诊断用）。
    pub fn team_id(&self) -> usize {
        match self {
            Identity::Live(t) => t.team.id.get(),
            Identity::Last(l) => l.team.get(),
        }
    }

    /// trap 帧物理地址：仅 Live 轴可读（本核在跑任务，帧必活）；Last → None。
    pub fn trap(&self) -> Option<PhysAddr> {
        match self {
            Identity::Live(t) => Some(frame_pa(t)),
            Identity::Last(_) => None,
        }
    }

    /// Live 轴内层身份（trap 路径消费：envcall / 用户缺页 / 空间翻译必有 running
    /// 任务；Last → None——无 running 任务时这些路径必然走不到，由调用方 expect）。
    pub fn live(&self) -> Option<&TaskIdent> {
        match self {
            Identity::Live(t) => Some(t),
            Identity::Last(_) => None,
        }
    }
}

/// 本核任务身份：seat 装槽时定型（Live 载荷 TaskIdent）；reap / park 无后继
/// 降级（Last 载荷两枚号）；未装槽 → None。无锁：写 = 本核 `Badge::{seat,shed}`
/// 的带标签 swap（AcqRel），读 = 本核 trap/panic（Acquire +
/// increment_strong_count）——载荷不可变 + 同 hart 程序序 ⇒ 非阻塞、不 panic、
/// 读恒有效，正常路径与崩溃现场同一入口。载荷类型自描述（标签位与载荷同行），
/// 无第二读点、无读写撕裂窗口。
///
/// 取本核只有一条路径：先查表（`boot::init` 先填每核直达指针、再发布表，故表在
/// ⇒ 指针在），再走 [`current`] 的 tp 直达。
pub fn ident() -> Option<Identity> {
    SCHEDULERS.get()?;
    current().badge.read()
}
