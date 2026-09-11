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

    match space.materialize(vaddr, PAGE_SIZE) {
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
/// 自愈，无需跨核 shootdown（见 `manager::asid` 清退语义）。只有 V 而权限不足则是
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
                // 已物化映射页缺页。步骤 1 的 re-walk 已把「PTE 满足访问却仍缺页」
                //（A/D 竞争 / 陈旧 TLB）判为 resolved，所以到此处 PTE **不满足**
                // 本次访问 ⇒ 权限违例（如写 R-only 的 cap⊆页表场景），或物化簿记与
                // 页表不一致。前者是 fault isolation 的正常触发，后者才是内核 bug。
                //
                // **"写只读私有页"只有一条来源，且不该被恢复**（用户裁决）：
                // 私有页的 W 只有 `Mprotect` 能翻回去（物化取 `map.flags`、`protect`
                // 重写叶 PTE，除此之外没有第二个写点），所以一次写缺页 = **程序自己
                // 写了自己标成只读的页**。内核在这里"帮它把 W 翻回来"等于把
                // `Mprotect` 从**边界**降级成**建议**——它同时是 `narrow` 收紧后的
                // 执法点。故此处继续判 false（fault isolation），不恢复。
                match space.translate(fault.addr) {
                    Some((_paddr, flags)) if !satisfies(flags, fault.kind) => {
                        // PTE 在但权限不足 ⇒ 真实访问违例，杀 task（非法越权访问）。
                        error!(
                            "permission violation on materialized map: {:?} at {:?}, pc={:#x}",
                            fault.kind, fault.addr, fault.pc
                        );
                    }
                    Some((_paddr, _flags)) => {
                        // PTE 满足访问却仍缺页（re-walk 未捕捉到）⇒ 簿记不一致。
                        error!(
                            "materialized map re-walk inconsistency: {:?} at {:?}, pc={:#x}",
                            fault.kind, fault.addr, fault.pc
                        );
                    }
                    None => {
                        // 物化簿记但页表无映射 ⇒ 内核 bug。
                        error!(
                            "materialized map missing PTE (kernel bug): {:?} at {:?}, pc={:#x}",
                            fault.kind, fault.addr, fault.pc
                        );
                    }
                }
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
