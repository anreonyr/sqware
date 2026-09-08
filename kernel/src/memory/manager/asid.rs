// asid — ASID 全生命周期：编号分配 + 跨核宿住登记 + TLB 清退。
//
// 三职责（原散在 asid.rs / evict.rs，此处合流——一个 ASID 从分配到释放的完整
// 生命周期集中在一处）：
//   allocate / deallocate — 位图编号分配（deallocate 先清退再还号，ASID 复用安全）
//   set_asid / vacate     — 本核宿住登记（per-hart lease 槽，纯 ASID；vacate=退驻）
//   shootdown             — 跨核 TLB 清退（SBI RFENCE，一次掩码调用）
//
// 命名：动词（allocate/deallocate/set_asid/vacate/shootdown）+ 宾语（ASID）。
//
// 硬不变量：
//   1. `shootdown(asid)` 返回即「该 ASID 的旧 TLB 条目在**所有驻留核**上已失效」
//      ——SBI RFENCE 同步阻塞，固件保证刷完才返。
//   2. `deallocate` 先 `shootdown` 再还位图——ASID 复用前必须无全系统残留。
//   3. `shootdown` 期间不得持任何关中断锁（Space::with 已在刷前释放锁）。

use fack::prelude::Error;
use sbi::ecall::SArgs;
use sbi::{self, fid};

use crate::lock::{Level, SpinLock};
use crate::machine;
use crate::memory::allocator::bitmap::BitmapAllocator;

use super::flush_asid;

/// ASID 位宽（satp 字段 16 位）：用户 ASID 1..=65535，ASID 0 保留给内核空间。
pub(crate) const ASID_BITS: u32 = 16;
/// 宿住槽的「退驻」哨兵（写在 lease 槽，表示本核当前不在用户态驻留任何 ASID）。
pub(crate) const VACANT: usize = 1 << ASID_BITS;

static ASID_ALLOCATOR: SpinLock<BitmapAllocator> =
    SpinLock::new_level(Level::Asid, BitmapAllocator::new(1, 65536, 1));

/// 空间身份 — ASID（`satp.ASID` 字段值）。
///
/// 0 保留给内核空间（[`Asid::kernel`]），只由 `SpaceBuilder::kernel()` 铸造；
/// 1..=65535 由 [`Asid::allocate`] 发放、[`deallocate`] 归还。分配器永不发 0，
/// 故 `is_kernel()` ⇔「这是内核空间」——陷阱路由（`trampoline.rs` 的
/// `__alltraps` 判别）、诊断展开、闭包任务与 `Space::drop` 均以此判别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asid(usize);

impl Asid {
    /// 内核空间的固定身份（ASID 0）。
    pub(crate) const fn kernel() -> Self {
        Self(0)
    }

    /// 从 `satp.ASID` 字段回读（切换/恢复路径用；值由内核自己写入，可信）。
    pub(crate) const fn from_raw(raw: usize) -> Self {
        Self(raw & 0xFFFF)
    }

    /// 分配一个独立 ASID（1..=65535）。耗尽时 panic。
    pub(crate) fn allocate() -> Self {
        let (asid, _) = ASID_ALLOCATOR
            .lock()
            .allocate(1)
            .expect("asid: 16-bit ASID space exhausted (65535 tasks)");
        Self(asid)
    }

    /// 裸值（写 satp / 组 wait·fence 键 / lease 槽）。
    pub fn get(self) -> usize {
        self.0
    }

    /// 是否内核空间（ASID 0）。
    pub fn is_kernel(self) -> bool {
        self.0 == 0
    }
}

/// 清退该 ASID 的全系统 TLB 残留后归还位图。double-free/未分配 panic。
///
/// 顺序契约：先 `shootdown`（清残留）再还位图——ASID 可复用后，任何核的残留
/// 条目会让新空间同 VA 命中旧映射。
///
/// # Errors
///
/// [`Deaf`] = RFENCE 失败；此时位图未动（ASID 不会被复用）。
pub fn deallocate(asid: Asid) -> Result<(), Deaf> {
    shootdown(asid)?;
    ASID_ALLOCATOR
        .lock()
        .deallocate(asid.get(), 1)
        .expect("asid: double-free or never-allocated");
    Ok(())
}

// ── 本核宿住登记（per-hart lease 槽）────────────────────────

/// 本核登记「当前驻留 ASID」：写本核 lease 槽（纯 ASID，无世代）。
///
/// 前置：`asid` 为本核即将驻留的合法空间身份（内核空间 = [`Asid::kernel`]）。
/// 调用点 = trap 入场/出场、trampoline restore、boot——每处都已保证本核 TLB 与
/// 将要驻留的 ASID 一致。
///
/// 幂等：重复登记同一 ASID 无害。
pub fn set_asid(asid: Asid) {
    machine::lease_store(asid.get());
}

/// 本核退驻：此后不被任何清退选中（写 VACANT）。幂等。
///
/// 调用点：关机/卧倒（conductor::halt、diagnose::halt::hunker）——终态核不再
/// 应答，必须先离册，否则发起方死等。
pub fn vacate() {
    machine::lease_store(VACANT);
}

// ── 跨核 TLB 清退（SBI RFENCE）────────────────────────────────

/// 清退 `asid`：返回即「该 ASID 的旧 TLB 条目在全系统不再被任何驻留核持有」。
///
/// 顺序契约：
///   ① 调用前 PTE 已写、Space 锁已释放（不变量 3）
///   ② 本核自刷（`flush_asid`）
///   ③ 扫名册得驻留 `asid` 的核 → 生成 hart_mask（不含本核，本核已在 ② 刷）
///   ④ 单次 `RemoteSfenceVmaAsid(mask, base, 0, 0, asid)`——固件同步等全部
///      目标核刷完才返。
///
/// 快路径：无他核驻留 → mask=0 → RFENCE 即返（零目标核）。
///
/// # Errors
///
/// [`Deaf`] = RFENCE 返回非 Success。
pub fn shootdown(asid: Asid) -> Result<(), Deaf> {
    // ② 本核自刷。
    // SAFETY: 页表已改完，刷后翻译即新映射。
    unsafe { flush_asid(asid.get()) };

    // ③ 扫名册生成 hart_mask（本核已在 ② 自刷，排除自己）。
    let me = machine::hart_id();
    let mut mask = 0usize;
    for hart in 0..machine::hart_count() {
        if hart == me {
            continue;
        }
        if machine::lease_load(hart) == asid.get() {
            mask |= 1usize << (hart % (usize::BITS as usize));
        }
    }

    // ④ 一次 RFENCE。hart_mask_base = 0（4 核均在首字，hartid 0..3）。
    let r = sbi::RfenceCall::new(fid::Rfence::RemoteSfenceVmaAsid)
        .args(SArgs {
            a0: mask,
            a1: 0, // hart_mask_base
            a2: 0, // start_addr：0 = 全地址空间
            a3: 0, // size：0 = 全地址空间
            a4: asid.get(),
            ..Default::default()
        })
        .call();
    r.map(|_| ()).map_err(|_| Deaf { asid: asid.get() })
}

// ── 错误 ────────────────────────────────────────────────────

/// 清退喊不应：RFENCE 失败（目标核未刷 / 固件拒绝）。
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[error("remote sfence failed for asid {asid}")]
pub struct Deaf {
    pub asid: usize,
}

// 编译期哨兵：ASID_BITS 与 satp 字段宽一致。
const _: () = assert!(ASID_BITS == 16);
