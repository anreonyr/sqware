// 跨核 TLB 清退（evict）— 租约册 + 清退协议。
//
// 租约册：每核一字（`machine::PerHart.lease`），记「本核此刻驻留哪个地址空间的
// 翻译」+ 世代计数。清退协议：改完页表的核刷自己 → 投 IPI 把驻留该 ASID 的他核
// 拉过一次刷点 → 等其租约字变值。IPI 无载荷正好够用：目标核**被拉进内核这件事
// 本身**（`__utrap` 整表刷 + 入场 `settle`）就是应答，接收侧一行不改。
//
// 词族：settle ↔ vacate（入住 ↔ 退租）、evict ↔ sweep（清退发起 ↔ 自扫应答）、
// tenant（名册单核读）。三个满足点见不变量 1。
//
// 硬不变量：
//   1. 世代递增 ⟺ 本核 TLB 已整表刷过（满足点：trap 入场后 / trap 出场前 /
//      `sweep` 内部）。
//   2. trap 出场「先 `settle`，后 `__restore` 的 sfence」——顺序反了，发起方
//      就可能在「名册上看不到你」的窗口里让你建立旧翻译。
//   3. `evict` 等到齐期间不得持任何关中断锁（`Space::with` 已在刷前释放锁）。

use core::time::Duration;

use fack::prelude::Error;
use sbi::scall::SArgs;
use sbi::{self, fid};

use crate::machine;
use crate::runtime::chrono::clock;

use super::{flush_all, flush_asid};

/// satp.ASID 字段宽（16 位）——租约槽再多一位放退租哨兵。
const ASID_BITS: u32 = 16;
/// 世代位段起点（租约槽之上）。
const GEN_SHIFT: u32 = ASID_BITS + 1;
/// 租约槽掩码（含哨兵位）。
const SLOT_MASK: usize = (1 << GEN_SHIFT) - 1;
/// 退租槽值（`PerHart.lease` 静态初值：未入册的核不得成为清退目标）。
pub(crate) const VACANT: usize = 1 << ASID_BITS;

/// 清退耐心：目标核未在此时限内到齐即判 [`Deaf`]。量纲取时间而非自旋次数
/// ——阈值不随主频漂移（参照：抢占量子 100 ms）。
const PATIENCE: Duration = Duration::from_millis(50);

/// 租约字：低 17 位 = 租约槽（`0..=0xFFFF` = 驻留该 ASID，[`VACANT`] = 退租），
/// 高 47 位 = 世代。
///
/// **只可判等，不可比大小**（刻意不实现 `PartialOrd`）：到齐判据是「字变了」，
/// 世代回绕由判等吸收。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Lease(usize);

impl Lease {
    /// 退租。
    pub const fn vacant() -> Self {
        Self(VACANT)
    }

    /// 驻留的 ASID；退租 → `None`。
    pub fn asid(self) -> Option<usize> {
        let slot = self.0 & SLOT_MASK;
        (slot != VACANT).then_some(slot)
    }

    /// 换租约槽 + 推进世代（纯函数）。
    fn renewed(self, slot: usize) -> Self {
        debug_assert!(slot <= VACANT, "lease: slot {slot:#x} out of range");
        Self(((self.0 >> GEN_SHIFT).wrapping_add(1) << GEN_SHIFT) | slot)
    }
}

/// 本核入住 `asid`（`0` = 内核空间）：推进世代 + 换租约。
///
/// 前置：调用点已整表刷过本核 TLB（不变量 1）。调用点 = `trap_handler` 首尾、
/// `boot_main`、`trampoline::restore`。
pub fn settle(asid: usize) {
    debug_assert!(
        asid <= u16::MAX as usize,
        "settle: asid {asid} beyond satp field"
    );
    let next = mine().renewed(asid);
    machine::lease_store(next.0);
}

/// 本核退租：此后不被任何清退选中。幂等。
///
/// 调用点：`conductor::halt` 登记屏障前、`diagnose::halt::hunker` 卧倒前
/// ——卧倒核永不应答，必须先从名册上消失，否则发起方死等。两处皆为终态
/// （不再回用户态），故退租无需推进世代。
pub fn vacate() {
    machine::lease_store(Lease::vacant().0);
}

/// 空闲核应答点：整表刷 + 推进世代（租约不变）。退租态 no-op（卧倒核不复活）。
///
/// 调用点：调度器 WFI 循环的 `wfi` 返回处——内核态 SIE=0 的空闲核不吃 trap，
/// 这是它唯一的刷点；以及 [`evict`] 的等待前自服务（防两核互等）。
pub fn sweep() {
    let cur = mine();
    let Some(asid) = cur.asid() else {
        return;
    };
    // SAFETY: S 态 sfence.vma 恒合法；整表刷后本核不持任何陈旧条目。
    unsafe { flush_all() };
    machine::lease_store(cur.renewed(asid).0);
}

/// 清退 `asid`：返回即「该 ASID 的旧 PTE 在全系统不再被任何 TLB 持有」。
///
/// 顺序契约（不可重排）：
///   ① 调用前 PTE 已写、Space 锁已释放（不变量 3）
///   ② 本核刷
///   ③ 逐核串行：取快照 → 该核仍驻留才投 IPI → **自服务一次** → 等其租约字变值
///
/// 逐核串行是为让「快照先于投递」成立：投递后再取快照会漏掉那一次响应。
///
/// 自服务（`sweep`）必须在**取快照之后**：两核同时清退内核空间（双方租约皆为
/// 内核）时，谁都不经过 trap 刷点，只等对方 = 互等。各自在等待前兑现一次自己的
/// 应答，至少一方必见对方变值，环即断开。
///
/// 快路径：无他核驻留 → 零 IPI、零自服务、**零时钟读**（`clock::init` 晚于
/// `unit::init`，boot 单核期恒走此路）。
///
/// # Errors
///
/// [`Deaf`] = 某核在 [`PATIENCE`] 内未到齐。
pub fn evict(asid: usize) -> Result<(), Deaf> {
    // SAFETY: 页表已改完，刷后翻译即新映射。
    unsafe { flush_asid(asid) };
    let me = machine::hart_id();
    let word = usize::BITS as usize;
    for hart in 0..machine::hart_count() {
        if hart == me {
            continue;
        }
        let snap = tenant(hart);
        if snap.asid() != Some(asid) {
            continue;
        }
        let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
            .args(SArgs {
                a0: 1 << (hart % word),
                a1: (hart / word) * word,
                ..Default::default()
            })
            .call();
        sweep();
        let deadline = clock::now().add(PATIENCE);
        while tenant(hart) == snap {
            if clock::now() > deadline {
                return Err(Deaf { hart, asid });
            }
            core::hint::spin_loop();
        }
    }
    Ok(())
}

/// 清退喊不应：某核未在 [`PATIENCE`] 内到齐（核号 + ASID 供 crash scene 定位）。
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[error("hart {hart} did not sweep asid {asid} within patience")]
pub struct Deaf {
    pub hart: usize,
    pub asid: usize,
}

/// 本核租约（自读自写，无跨核竞争）。
fn mine() -> Lease {
    Lease(machine::lease_load(machine::hart_id()))
}

/// 名册单核读（他核视角）。
fn tenant(hart: usize) -> Lease {
    Lease(machine::lease_load(hart))
}
