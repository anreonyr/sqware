// 缺页异常处理
//
// 替换 trap.rs 中的 panic，提供结构化的缺页诊断和处理框架。
// 当前阶段：
//   - 内核缺页 → fatal（内核页必须预映射）
//   - 用户缺页 → 匿名页分配（懒分配）

use fack::prelude::Error;
use log::{error, info};
use riscv::register::{scause, sepc, stval};

use crate::memory::PAGE_SIZE;

use super::{
    addr::VirtAddr,
    entry::PteFlags,
    space::{MapKind, Space},
};

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
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
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

/// 为用户缺页解析匿名物理页。
///
/// 按映射权限从 frame 分配器取一页并映射到缺页地址。
fn resolve_anonymous(fault: &PageFault, space: &Space, flags: PteFlags) -> bool {
    let vaddr = fault.addr.page_align();
    // A/D 必须设置，否则硬件可能再次缺页
    let flags = flags | PteFlags::A | PteFlags::D;

    match space.page_fault(vaddr, PAGE_SIZE, flags) {
        Ok(()) => {
            // map 内部已按空间 ASID 局部刷 TLB（只失效本空间旧条目）。
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

/// 处理缺页异常。
///
/// 返回 `true` 表示已解决（可以 sret 重试），`false` 表示无法处理。
///
/// # 处理策略
///
/// 1. Re-walk 页表 — 排除 A/D 位竞争
/// 2. 用户地址 → 查 Map：Anonymous 分配零页，Reserved/无 Map 返回 false
/// 3. 内核地址 → fatal（内核页必须预映射）
pub fn handle_page_fault(fault: &PageFault, space: &Space) -> bool {
    // 0. COW：写缺页命中共享（Borrowed）页 → 分裂为私有可写（保留共享内容）。
    //    必须先于 Re-walk：共享页 PTE 有效（置了 A/D、清了 W），若不拦会在
    //    步骤 1 被当成 A/D 竞争而重试 → 无限缺页循环。
    if fault.addr.is_user()
        && matches!(fault.kind, FaultKind::Store)
        && space.is_borrowed(fault.addr)
    {
        return space.to_mut(fault.addr).is_ok();
    }
    // 1. Re-walk 页表 (A/D 位竞争检查)
    if let Some((_paddr, flags)) = space.translate(fault.addr)
        && flags.contains(PteFlags::V)
    {
        // 映射存在 — 可能是 A/D 位的瞬时竞争，直接重试
        info!(
            "page fault resolved by re-walk: {:?} at {:?}",
            fault.kind, fault.addr
        );
        return true;
    }

    // 2. 用户地址 → 查 Map（常数表 + 窗口子表）
    if fault.addr.is_user() {
        if let Some(map) = space.resolve(fault.addr) {
            match map.kind {
                MapKind::Anonymous => {
                    return resolve_anonymous(fault, space, map.flags);
                }
                MapKind::Reserved => {
                    error!(
                        "reserved region access: {:?} at {:?}, pc={:#x}",
                        fault.kind, fault.addr, fault.pc
                    );
                    return false;
                }
            }
        } else {
            error!(
                "no map for user page fault: {:?} at {:?}, pc={:#x}",
                fault.kind, fault.addr, fault.pc
            );
            return false;
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
