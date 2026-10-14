use alloc::sync::Arc;

use env::{MemoryCall, MemoryFail};

use crate::core::memory::manager::Access;
use crate::core::memory::manager::Memory;
use crate::core::memory::manager::addr::VirtAddr as KVirt;
use crate::core::work::mail::pole::PoleOperation;
use crate::core::work::unit::task::TaskIdent;
use crate::machine::switcher::context::TrapContext;

use super::outcome::answer;

pub(super) fn dispatch(frame: &mut TrapContext, call: MemoryCall, ident: &Arc<TaskIdent>) {
    answer(frame, execute(call, ident));
}

fn execute(call: MemoryCall, ident: &Arc<TaskIdent>) -> Result<usize, MemoryFail> {
    use crate::core::work::pend::{self, Permission};
    use crate::core::work::room::conductor::current;
    use crate::core::work::room::scheduler::roster::muster;
    let raw_size = match call {
        MemoryCall::Allocate { size }
        | MemoryCall::Deallocate { size, .. }
        | MemoryCall::Mmap { size, .. }
        | MemoryCall::Munmap { size, .. }
        | MemoryCall::Mprotect { size, .. } => size,
    };
    // 尺寸口径在授权之前：非 basic 报 `NotAligned`，basic 报 `Denied`，两者不换码。
    let basic = matches!(
        call,
        MemoryCall::Allocate { .. } | MemoryCall::Deallocate { .. }
    );
    let size = if basic {
        Memory::basic_bytes(raw_size)?
    } else {
        Memory::exact_bytes(raw_size)?
    };
    let team_id = match call {
        MemoryCall::Mmap { team, .. }
        | MemoryCall::Munmap { team, .. }
        | MemoryCall::Mprotect { team, .. } => team,
        _ => env::TeamId::new(0),
    };
    let caller = current()
        .running_task()
        .or_else(|| muster(ident.id).and_then(|task| task.upgrade()));
    let target = if team_id.get() == 0 {
        ident.team.clone()
    } else {
        caller
            .as_ref()
            .and_then(|task| task.clan(team_id))
            .ok_or(MemoryFail::Denied)?
    };
    let _construction = if team_id.get() != 0 {
        Some(target.operation().ok_or(MemoryFail::Busy)?)
    } else {
        None
    };
    if team_id.get() != 0 && !target.constructing() {
        return Err(MemoryFail::Denied);
    }
    let space = &target.space;
    match call {
        MemoryCall::Allocate { .. } => {
            let caller = caller.as_ref().ok_or(MemoryFail::Denied)?;
            let prepared = caller
                .pend
                .creations
                .prepare()
                .map_err(|_| MemoryFail::OoM)?;
            // 走字节入口（同一条 basic 口径）：`Layout` 表示不了的线上尺寸不在这里
            // 被强迫失败。
            let access = Access::READ | Access::WRITE;
            let mapping = space.memory.allocate_bytes(raw_size, access, None)?;
            let address = mapping.address().as_usize();
            prepared.commit(&mapping.map);
            Ok(address)
        }
        MemoryCall::Deallocate { addr, .. } => {
            if !Memory::user_range(addr.get(), size) {
                return Err(MemoryFail::Denied);
            }
            let caller = caller.as_ref().ok_or(MemoryFail::Denied)?;
            let map = caller
                .pend
                .creations
                .locate(KVirt::wrap(addr.get()), size)
                .ok_or(MemoryFail::Denied)?;
            let creation = caller.pend.creations.claim(&map).map_err(|error| {
                if error == env::PieFail::Busy {
                    MemoryFail::Busy
                } else {
                    MemoryFail::Denied
                }
            })?;
            space.memory.retry_invalidation()?;
            if map.is_live() {
                space.memory.unmap_range(KVirt::wrap(addr.get()), size)?;
            }
            creation.commit();
            Ok(0)
        }
        MemoryCall::Mmap {
            at,
            pole,
            offset,
            flags,
            ..
        } => {
            let access = Access::from_wire(flags).ok_or(MemoryFail::Denied)?;
            if !access.contains(Access::READ) || access.contains(Access::WRITE | Access::EXECUTE) {
                return Err(MemoryFail::Denied);
            }
            let source = if pole == env::PieToken::NONE {
                None
            } else {
                let caller = caller.as_ref().ok_or(MemoryFail::Denied)?;
                let pie = pend::locate(caller, pole).ok_or(MemoryFail::Denied)?;
                let meta = pie.mail().pole().ok_or(MemoryFail::Denied)?;
                Some((pie, meta))
            };
            let _operation = match &source {
                Some((_, meta)) => Some(PoleOperation::acquire(meta).ok_or(MemoryFail::Busy)?),
                None => None,
            };
            let mut private = false;
            let mut limit = access;
            if let Some((pie, meta)) = &source {
                if !pie.alive() || !meta.owned() || meta.reserved() != 0 {
                    return Err(MemoryFail::Denied);
                }
                let permission = pie.permission();
                if !permission.contains(Permission::FETCH)
                    || access.contains(Access::WRITE) && !permission.contains(Permission::STORE)
                    || team_id.get() != 0 && !permission.contains(Permission::VEST)
                {
                    return Err(MemoryFail::Denied);
                }
                super::pie::usable(pie).map_err(|_| MemoryFail::Denied)?;
                Memory::frame_offset(offset)?;
                if offset.checked_add(size).is_none_or(|end| end > meta.size()) {
                    return Err(MemoryFail::NotAligned);
                }
                if team_id.get() == 0 {
                    if access.contains(Access::EXECUTE) {
                        return Err(MemoryFail::Denied);
                    }
                    limit = if permission.contains(Permission::STORE) {
                        Access::READ | Access::WRITE
                    } else {
                        Access::READ
                    };
                } else if permission.contains(Permission::ONLY) {
                    if pie.sire().is_some()
                        || pie.heir().is_some()
                        || meta.owner() != ident.id
                        || !meta.exclusive()
                        || offset != 0
                        || size != meta.size()
                        || access.contains(Access::EXECUTE)
                        || meta.mapped()
                    {
                        return Err(MemoryFail::Busy);
                    }
                    target.reserve_staging(1).map_err(|_| MemoryFail::OoM)?;
                    private = true;
                } else if !meta.readonly() {
                    return Err(MemoryFail::Busy);
                }
                if team_id.get() == 0 || private {
                    meta.reserve_record().map_err(|_| MemoryFail::OoM)?;
                }
            } else if offset != 0 || access.contains(Access::EXECUTE) {
                return Err(MemoryFail::Denied);
            }
            let at = if at.get() == 0 {
                None
            } else {
                Memory::fixed_range(KVirt::wrap(at.get()), size)?;
                Some(KVirt::wrap(at.get()))
            };
            let mapping = match &source {
                Some((_, meta)) => {
                    let canonical = meta.canonical_map().ok_or(MemoryFail::Denied)?;
                    space
                        .memory
                        .view_record(&canonical, offset, size, limit, access, at)?
                }
                None => space.memory.allocate_lazy_bytes(size, limit, access, at)?,
            };
            let address = mapping.address().as_usize();
            if let Some((pie, _)) = &source {
                let permission = pie.permission();
                if !pie.alive()
                    || !permission.contains(Permission::FETCH)
                    || access.contains(Access::WRITE) && !permission.contains(Permission::STORE)
                {
                    let _ = space.memory.dispose(mapping);
                    return Err(MemoryFail::Denied);
                }
            }
            if access.contains(Access::EXECUTE) {
                if let Err(error) = crate::core::memory::manager::asid::sync_instructions() {
                    let _ = space.memory.dispose(mapping);
                    return Err(error.into());
                }
            }
            if let Some((pie, meta)) = &source {
                if private {
                    let staged_map = mapping.map.clone();
                    if let Err(mapping) =
                        meta.record(pie.token(), space, mapping, Some(target.id), false)
                    {
                        let _ = space.memory.dispose(mapping);
                        return Err(MemoryFail::Denied);
                    }
                    target.push_staging(crate::core::work::unit::team::Staging {
                        token: pie.token(),
                        meta: meta.clone(),
                        map: staged_map,
                    });
                    pend::notify(ident.id, pole);
                } else if team_id.get() == 0 {
                    if let Err(mapping) = meta.record(pie.token(), space, mapping, None, false) {
                        let _ = space.memory.dispose(mapping);
                        return Err(MemoryFail::Denied);
                    }
                }
            }
            Ok(address)
        }
        MemoryCall::Munmap { addr, .. } => {
            if team_id.get() == 0 {
                let maps = space.memory.mappings_in(KVirt::wrap(addr.get()), size)?;
                if let Some(caller) = &caller {
                    let pies = caller.pend.pies.lock();
                    if maps.iter().any(|map| {
                        pies.iter().any(|pie| {
                            pie.snapshot()
                                .mail()
                                .pole()
                                .is_some_and(|meta| meta.opened(map))
                        })
                    }) {
                        return Err(MemoryFail::Denied);
                    }
                }
            }
            let staged = target
                .find_staging(|item| item.map.initial_range() == (KVirt::wrap(addr.get()), size));
            if let Some(index) = staged {
                let entry = target.staged_at(index).ok_or(MemoryFail::Denied)?;
                let (meta, token) = (entry.meta.clone(), entry.token);
                let _operation = PoleOperation::acquire(&meta).ok_or(MemoryFail::Busy)?;
                space.memory.unmap_range(KVirt::wrap(addr.get()), size)?;
                meta.detach_staging(token, target.id);
                target.remove_staging(index);
                pend::notify(meta.owner(), token);
                return Ok(0);
            }
            if target
                .find_staging(|item| {
                    let (start, len) = item.map.initial_range();
                    addr.get() < start.as_usize() + len
                        && start.as_usize() < addr.get().saturating_add(size)
                })
                .is_some()
            {
                return Err(MemoryFail::Busy);
            }
            space.memory.unmap_range(KVirt::wrap(addr.get()), size)?;
            Ok(0)
        }
        MemoryCall::Mprotect { addr, flags, .. } => {
            Memory::fixed_range(KVirt::wrap(addr.get()), size)?;
            let access = Access::from_wire(flags).ok_or(MemoryFail::Denied)?;
            // 空权限与没有 R 的 W 都拒；RWX 仍放行（不与 Mmap 统一成更严的 W^X）。
            let write_without_read =
                access.contains(Access::WRITE) && !access.contains(Access::READ);
            if access.is_empty() || write_without_read {
                return Err(MemoryFail::Denied);
            }
            space
                .memory
                .protect_range(KVirt::wrap(addr.get()), size, access)?;
            Ok(0)
        }
    }
}
