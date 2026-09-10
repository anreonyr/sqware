// 本核身份槽（core::ident）— 身份两态、带标签指针与唯一读法 `ident()`。
//
// 载荷两态：Live = 本核**在跑**任务（trap 可信）；Last = 末次身份记录（id / 角色名 /
// 域名字；trap **不可信**且类型上不可读）。标签位与指针在**同一个原子字**里——载荷
// 类型自描述，读侧无需第二读点（两个写点无读撕裂窗口）。
//
// 计数协议：槽持有一份 Arc 强引用。写只有三个方法——[`Badge::seat`]（装 Live）/
// [`Badge::shed`]（换 Last）/ [`Badge::clear`]（清空），旧载荷的归还只有一处实现
// （`Badge::reclaim`）：「按标签位收回归还」不再在三个写点各抄一遍。
// 读 = `Badge::read`（Acquire + increment_strong_count）。
// 同 hart 单写单读 + 载荷不可变 ⇒ 无锁；跨核读是 UB（槽私有，只经 [`ident`] 触及，
// 而它读的必是本核的槽）。

use alloc::sync::Arc;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::memory::manager::addr::PhysAddr;
use crate::work::unit::task::TaskIdent;

use super::hart::frame_pa;
use super::table::{SCHEDULERS, current};

/// 身份槽载荷类型标签（bit0）：0 = TaskIdent（在跑任务），1 = LastIdent（末次
/// 记录）。标签与指针同一原子字——载荷类型自描述，读侧无需第二读点。
const LAST_TAG: usize = 1;

/// 末次身份记录：降级时从 TaskIdent 复制（id / 角色名 / **域名字**），
/// **不含 team/space/trap**——团队 Arc 借此归零即回收整个地址空间。
pub struct LastIdent {
    pub(crate) id: usize,
    pub(crate) name: &'static str,
    /// 域名字（程序身份；内联定长，故仍不持 Team）。
    pub(crate) team: env::Name,
}

/// 身份槽载荷：Live = 本核**在跑**任务（trap 可信）；Last = 末次身份记录
/// （id/name/符号表；trap **不可信**且类型上不可读）。trap 只经 Live 轴暴露——
/// 悬垂帧读取在类型层不可表达。
pub enum Identity {
    Live(Arc<TaskIdent>),
    Last(Arc<LastIdent>),
}

/// 本核身份槽（工牌）：谁在本核上跑。**唯一写点 = 三个方法**，回收只有一个实现。
///
/// 字段私有：读只经 [`ident`]，写只经 `seat` / `shed` / `clear`（调用点都在
/// `core::hart`），于是「带标签指针 + arc 计数交换」这套协议只活在本文件里。
pub(super) struct Badge {
    raw: AtomicPtr<()>,
}

impl Badge {
    /// 空槽（未装牌）。
    pub(super) fn new() -> Badge {
        Badge {
            raw: AtomicPtr::new(core::ptr::null_mut()),
        }
    }

    /// 装牌：写入 Live 载荷（本核在跑任务）。`Arc::into_raw` 交出克隆的计数给槽持有。
    pub(super) fn seat(&self, ident: &Arc<TaskIdent>) {
        let prev = self.raw.swap(
            Arc::into_raw(ident.clone()).cast_mut() as *mut (),
            Ordering::AcqRel,
        );
        Self::reclaim(prev);
    }

    /// 换牌：Live → Last（本核在跑任务离核且不接续装槽：reap / park 无后继）。
    /// Last 只留符号化最小集（id/name/域名字），**不持有团队/空间**——团队 Arc 借此
    /// 归零即回收，地址空间不再被 idle 核钉住（关机零泄漏审计与「末次符号化」兼得）。
    pub(super) fn shed(&self, ident: &Arc<TaskIdent>) {
        let last = Arc::new(LastIdent {
            id: ident.id,
            name: ident.name,
            team: ident.team.name(),
        });
        let prev = self.raw.swap(
            (Arc::into_raw(last) as usize | LAST_TAG) as *mut (),
            Ordering::AcqRel,
        );
        // SAFETY: 降级只在拥有在跑任务时发生——旧载荷必为未标签 TaskIdent。
        debug_assert_eq!(prev as usize & LAST_TAG, 0, "shed 旧载荷带标签");
        Self::reclaim(prev);
    }

    /// 清空槽载荷（关机基线审计前调用，否则每 hart 末次 LastIdent 计入块差集误报
    /// 泄漏——已实证：4 hart = 4 个 48B 假泄漏）。
    pub(super) fn clear(&self) {
        Self::reclaim(self.raw.swap(core::ptr::null_mut(), Ordering::AcqRel));
    }

    /// 读：None = 未装槽。载荷类型按标签位判定（标签与指针原子同行，无撕裂）。
    fn read(&self) -> Option<Identity> {
        let raw = self.raw.load(Ordering::Acquire) as usize;
        if raw == 0 {
            return None;
        }
        if raw & LAST_TAG != 0 {
            // SAFETY: 标签标记 = LastIdent 载荷；槽持有者对 p 保有一份计数（Acquire
            // 与存入侧 AcqRel 配对，记录数据已发布）。同 hart 程序序下本核读时无
            // 并发 swap——increment 后再 from_raw 克隆，归还时计数一致。
            let p = (raw & !LAST_TAG) as *const LastIdent;
            unsafe {
                Arc::increment_strong_count(p);
                Some(Identity::Last(Arc::from_raw(p)))
            }
        } else {
            // SAFETY: 未标签 = TaskIdent 载荷；计数协议同上一臂（Arc 数据经 AcqRel
            // swap 发布）。
            let p = raw as *const TaskIdent;
            unsafe {
                Arc::increment_strong_count(p);
                Some(Identity::Live(Arc::from_raw(p)))
            }
        }
    }

    /// 归还旧载荷持有的那份计数（三个写点唯一的回收实现）。
    fn reclaim(prev: *mut ()) {
        if prev.is_null() {
            return;
        }
        let prev = prev as usize;
        if prev & LAST_TAG != 0 {
            // SAFETY: 标签标记 = LastIdent 载荷；swap 取走后槽对其不再持有，此处
            // from_raw 收回该份计数并释放（同 hart 程序序，无并发的本槽读写）。
            unsafe {
                drop(Arc::from_raw((prev & !LAST_TAG) as *const LastIdent));
            }
        } else {
            // SAFETY: 未标签 = TaskIdent 载荷，回收协议同上一臂。
            unsafe {
                drop(Arc::from_raw(prev as *const TaskIdent));
            }
        }
    }
}

impl Identity {
    pub fn id(&self) -> usize {
        match self {
            Identity::Live(t) => t.id,
            Identity::Last(l) => l.id,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Identity::Live(t) => t.name,
            Identity::Last(l) => l.name,
        }
    }

    /// 域名字（程序身份；诊断用）。
    pub fn team_name(&self) -> env::Name {
        match self {
            Identity::Live(t) => t.team.name(),
            Identity::Last(l) => l.team,
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
/// 降级（Last 载荷 LastIdent）；未装槽 → None。无锁：写 = 本核 `Badge::{seat,shed}`
/// 的带标签指针 swap（AcqRel），读 = 本核 trap/panic（Acquire +
/// increment_strong_count）——载荷不可变 + 同 hart 程序序 ⇒ 非阻塞、不 panic、
/// 读恒有效，正常路径与崩溃现场同一入口。载荷类型自描述（标签位与指针同行），
/// 无第二读点、无读写撕裂窗口。
///
/// 取本核只有一条路径：先查表（`boot::init` 先填每核直达指针、再发布表，故表在
/// ⇒ 指针在），再走 [`current`] 的 tp 直达。
pub fn ident() -> Option<Identity> {
    SCHEDULERS.get()?;
    current().badge.read()
}
