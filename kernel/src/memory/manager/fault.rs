// 缺页异常处理。
//
// 当前阶段：
//   - 内核缺页 → fatal（内核页必须预映射）
//   - 用户缺页 → 匿名页分配（懒分配）

use fack::prelude::Error;
use log::{error, info};
use riscv::register::{scause, sepc, stval};

use crate::memory::PAGE_SIZE;

use super::{addr::VirtAddr, entry::PteFlags};
use crate::work::unit::space::{PendingState, Space};

/// 从机器 CSR 捕获的缺页信息。
#[derive(Debug)]
pub struct PageFault {
    /// 引发缺页的虚拟地址（来自 stval）
    pub addr: VirtAddr,
    /// 缺页时的程序计数器（来自 sepc）
    pub pc: usize,
    /// 缺页类型
    pub kind: FaultKind,
}

/// 缺页类型
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum FaultKind {
    /// 指令缺页 (scause = 12)
    #[error("Execute OOM Instruction")]
    Instruction,
    /// 加载缺页 (scause = 13)
    #[error("Load OOM Data")]
    Load,
    /// 存储/AMO 缺页 (scause = 15)
    #[error("Store OOM Data")]
    Store,
}

impl PageFault {
    /// 从当前 CSR 状态捕获缺页信息。
    ///
    /// 仅在 trap handler 内调用。
    pub unsafe fn capture() -> Self {
        let code = scause::read().code();
        let kind = match code {
            12 => FaultKind::Instruction,
            13 => FaultKind::Load,
            15 => FaultKind::Store,
            _ => panic!("capture() called on non-page-fault scause={}", code),
        };

        Self {
            addr: VirtAddr::from_raw(stval::read()),
            pc: sepc::read(),
            kind,
        }
    }
}

/// 为用户缺页解析匿名物理页（flags 由 materialize 从映射自取：`map.flags | A | D`）。
fn resolve_anonymous(fault: &PageFault, space: &Space) -> bool {
    let vaddr = fault.addr.page_align();

    match space.materialize_map(vaddr, PAGE_SIZE) {
        Ok(()) => {
            info!(
                "resolved page fault: allocated anon page for {:?} at {:?}",
                fault.kind, vaddr
            );
            true
        }
        Err(e) => {
            error!("failed to resolve page fault: {:?}", e);
            false
        }
    }
}

/// 该 PTE 权限是否满足此次访问（Instruction→X / Load→R / Store→W）。
///
/// 陈旧条目的判据：PTE 已满足本次访问却仍缺页 ⇒ 硬件持旧 TLB 条目（或 A/D 位
/// 瞬时竞争），重试即成——远核「新增 / 放宽」类页表变更靠这条 + trap 两侧整表刷
/// 自愈，无需跨核清退（见 `manager::evict` 模块头）。只有 V 而权限不足则是
/// **真实违例**，不得判 resolved（否则重试立即再缺页 = 无限缺页循环）。
fn satisfies(flags: PteFlags, kind: FaultKind) -> bool {
    let need = match kind {
        FaultKind::Instruction => PteFlags::X,
        FaultKind::Load => PteFlags::R,
        FaultKind::Store => PteFlags::W,
    };
    flags.contains(PteFlags::V | need)
}

/// 处理缺页异常。
///
/// 返回 `true` 表示已解决（可以 sret 重试），`false` 表示无法处理。
///
/// # 处理策略
///
/// 1. Re-walk 页表 — 排除陈旧 TLB 条目 / A-D 位竞争
/// 2. 用户地址 → 查 Map：Anonymous 分配零页，Reserved/无 Map 返回 false
/// 3. 内核地址 → fatal（内核页必须预映射）
pub fn handle_page_fault(fault: &PageFault, space: &Space) -> bool {
    // 0. COW：写缺页命中共享（Shared）页 → 分裂为私有可写（保留共享内容）。
    //    必须先于 Re-walk：共享页 PTE 有效（置了 A/D、清了 W），若不拦会在
    //    步骤 1 被当成 A/D 竞争而重试 → 无限缺页循环。
    //    注：Lazy 区在 `SpaceInner::share` 时未触页被跳过（见其注释），
    //    首次写缺页仍走 own 分裂，非 COW。
    if fault.addr.is_user() && matches!(fault.kind, FaultKind::Store) && space.is_shared(fault.addr)
    {
        return space.own(fault.addr, PAGE_SIZE).is_ok();
    }
    // 1. Re-walk 页表（陈旧 TLB 条目 / A-D 位竞争：PTE 已满足本次访问）
    if let Some((_paddr, flags)) = space.translate(fault.addr)
        && satisfies(flags, fault.kind)
    {
        info!(
            "page fault resolved by re-walk: {:?} at {:?}",
            fault.kind, fault.addr
        );
        return true;
    }

    // 2. 用户地址 → 查映射物化态（Lazy 物化零页；Guard 预留触碰；Absent 无映射）
    if fault.addr.is_user() {
        match space.pending_state(fault.addr) {
            PendingState::Lazy => {
                return resolve_anonymous(fault, space);
            }
            PendingState::Guard => {
                error!(
                    "reserved region access: {:?} at {:?}, pc={:#x}",
                    fault.kind, fault.addr, fault.pc
                );
                return false;
            }
            PendingState::Materialized => {
                // 全物化映射（含借用）不该缺页：re-walk 已排除 A/D 竞争，到此处 =
                // 内核簿记错误。
                error!(
                    "page fault on materialized map: {:?} at {:?}, pc={:#x}",
                    fault.kind, fault.addr, fault.pc
                );
                return false;
            }
            PendingState::Absent => {
                error!(
                    "no map for user page fault: {:?} at {:?}, pc={:#x}",
                    fault.kind, fault.addr, fault.pc
                );
                return false;
            }
        }
    }

    // 3. 内核地址 → fatal
    error!(
        "unhandled kernel page fault: {:?} at {:?}, pc={:#x}",
        fault.kind, fault.addr, fault.pc
    );
    error!("kernel page fault — this is a bug (kernel pages must be pre-mapped)");
    false
}
