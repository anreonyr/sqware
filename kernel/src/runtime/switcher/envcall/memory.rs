// Memory 域（class 2）—— 用户堆与映射：五格 + **唯一一处** `MapError → MemoryFail` 折算。
//
// 与 pie/mail/tole 同形：`dispatch` 只把本域的臂摆在一处，`mod.rs` 只做 decode 与分派。
//
// **错码不再是裸字面量**：从前这五格写 `usize::MAX`（= -1，把 `MapError` 整个抹掉），
// 今天每格答的是 `MemoryFail` 里那一枚——号在域内自 `-1` 起（`Denied` 恰是 `-1`，
// 故"簿记对不上"那三格的线上字节与从前一字不差）。
//
// **同一个内核错误可以落进两张词表**：`MapError` 在 `Unit` 域的 `Spawn` 那一格另有一处
// 折算（`map_err`）——两域的词汇面不同，故各说各的话，不共用码。

use alloc::sync::Arc;

use env::{MemoryCall, MemoryFail};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::memory::manager::entry::PteFlags;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::unit::space::window::{HeapWindow, ShareWindow};
use crate::work::unit::space::{Pending, PendingState};
use crate::work::unit::task::TaskIdent;

use super::ret_err;

/// 映射错误 → Memory 域的词汇。**穷尽 match**：`MapError` 多一枚变体就编不过。
///
/// 下面那三枚**从这五格到不了**（它们属装载 / 借还那条路）：真到了就是内核的 bug，
/// 故不写兜底——写兜底会把"到不了的格子"变成一句静默的谎。
impl From<MapError> for MemoryFail {
    fn from(e: MapError) -> Self {
        match e {
            MapError::OutOfMemory => MemoryFail::OoM,
            MapError::NotAligned => MemoryFail::NotAligned,
            MapError::NoRegion => MemoryFail::NoRegion,
            MapError::AlreadyMapped => MemoryFail::AlreadyMapped,
            MapError::WidenDenied => MemoryFail::WidenDenied,
            MapError::NotMapped | MapError::SegmentMismatch | MapError::DramOverlap => {
                unreachable!("Memory 五格不该见到这一枚 MapError")
            }
        }
    }
}

/// 本域的臂。**从不挂起**：五格要么当场答成功载荷，要么当场答一枚 `MemoryFail`。
pub(super) fn dispatch(frame: &mut TrapContext, call: MemoryCall, ident: &Arc<TaskIdent>) {
    match call {
        MemoryCall::Allocate { size } => {
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            match HeapWindow::allocate(&ident.team.space, size) {
                Ok(span) => frame.gpr.set_x(Gprs::A0, span.va.as_usize()),
                // 段耗尽 / 帧耗尽 ⇒ `OoM`；本域没有 `user` 段 ⇒ `NoRegion`（不变量）。
                Err(e) => {
                    ret_err(frame, MemoryFail::from(e));
                }
            }
        }
        MemoryCall::Deallocate { addr, size } => {
            let addr = addr.get();
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            let ok = HeapWindow::deallocate(&ident.team.space, KVirt::from_raw(addr), size);
            frame.gpr.set_x(
                Gprs::A0,
                if ok { 0 } else { MemoryFail::Denied.code() as usize },
            );
        }
        MemoryCall::Mmap { size, at } => {
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            let fixed = at.get();
            let va = {
                let s = &ident.team.space;
                if fixed == 0 {
                    ShareWindow::mmap(s, size).map(|span| span.va)
                } else {
                    let flags = s.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W);
                    s.map(KVirt::from_raw(fixed), size, flags, Some(Pending::Lazy))
                        .map(|()| KVirt::from_raw(fixed))
                }
            };
            match va {
                Ok(va) => frame.gpr.set_x(Gprs::A0, va.as_usize()),
                // 窗口自选：段不足 ⇒ `OoM`；定点：未对齐 ⇒ `NotAligned`、已占 ⇒ `AlreadyMapped`。
                Err(e) => {
                    ret_err(frame, MemoryFail::from(e));
                }
            }
        }
        MemoryCall::Munmap { addr, size } => {
            let addr = KVirt::from_raw(addr.get());
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            let ok = {
                let s = &ident.team.space;
                if ShareWindow::munmap(s, addr, size) {
                    true
                } else if s.pending_state(addr) != PendingState::Absent {
                    // 部分覆盖时 `unmap` 要分裂、分裂要造图 ⇒ 可能答 `OutOfMemory`
                    // （簿记一字未动）——用户面照旧是"没拆成"。
                    s.unmap(addr, size).is_ok()
                } else {
                    false
                }
            };
            frame.gpr.set_x(
                Gprs::A0,
                if ok { 0 } else { MemoryFail::Denied.code() as usize },
            );
        }
        MemoryCall::Mprotect { addr, size, flags } => {
            let addr = KVirt::from_raw(addr.get());
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            // 校验式：非法位 → 拒绝（不再 from_bits_truncate 静默截断）。
            let ok = match PteFlags::from_bits(flags) {
                Some(f) => ident.team.space.protect(addr, size, f).is_ok(),
                None => false,
            };
            frame.gpr.set_x(
                Gprs::A0,
                if ok { 0 } else { MemoryFail::Denied.code() as usize },
            );
        }
    }
}
