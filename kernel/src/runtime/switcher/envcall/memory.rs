use alloc::sync::Arc;

use env::{MemoryCall, MemoryFail};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::unit::space::Space;
use crate::work::unit::task::TaskIdent;

use super::ret_err;

impl From<MapError> for MemoryFail {
    fn from(e: MapError) -> Self {
        match e {
            MapError::OutOfMemory => MemoryFail::OoM,
            MapError::NotAligned => MemoryFail::NotAligned,
            MapError::NoRegion => MemoryFail::NoRegion,
            MapError::AlreadyMapped => MemoryFail::AlreadyMapped,
            MapError::WidenDenied => MemoryFail::WidenDenied,
            MapError::Shootdown => MemoryFail::Busy,
            MapError::Pinned => MemoryFail::Denied,
            MapError::NotMapped | MapError::DramOverlap => MemoryFail::Denied,
        }
    }
}

pub(super) fn dispatch(frame: &mut TrapContext, call: MemoryCall, ident: &Arc<TaskIdent>) {
    let result = execute(call, ident);
    match result {
        Ok(value) => frame.gpr.set_x(Gprs::A0, value),
        Err(error) => {
            ret_err(frame, error);
        }
    }
}

fn execute(call: MemoryCall, ident: &Arc<TaskIdent>) -> Result<usize, MemoryFail> {
    use crate::memory::manager::{Frame, entry::PteFlags};
    use crate::work::pend::{self, Permission};
    use crate::work::room::scheduler::core::{current, muster};
    use crate::work::unit::space::RegionId;
    use alloc::vec::Vec;
    let raw_size = match call {
        MemoryCall::Allocate { size }
        | MemoryCall::Deallocate { size, .. }
        | MemoryCall::Mmap { size, .. }
        | MemoryCall::Munmap { size, .. }
        | MemoryCall::Mprotect { size, .. } => size,
    };
    let basic = matches!(
        call,
        MemoryCall::Allocate { .. } | MemoryCall::Deallocate { .. }
    );
    let size = if basic {
        raw_size
            .max(1)
            .checked_next_multiple_of(PAGE_SIZE)
            .ok_or(MemoryFail::Denied)?
    } else {
        if raw_size == 0 || !raw_size.is_multiple_of(PAGE_SIZE) {
            return Err(MemoryFail::NotAligned);
        }
        raw_size
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
            let mut frames = Vec::new();
            frames
                .try_reserve_exact(size / PAGE_SIZE)
                .map_err(|_| MemoryFail::OoM)?;
            for _ in 0..size / PAGE_SIZE {
                frames.push(Frame::allocate()?);
            }
            space.retry_invalidation()?;
            let reservation = space.allocate_low(RegionId::Normal, size)?;
            let rw = PteFlags::R | PteFlags::W;
            let mapping = space.map_frames(
                reservation,
                frames,
                rw | PteFlags::X,
                rw | PteFlags::A | PteFlags::D,
            )?;
            let address = mapping.reservation.start().as_usize();
            prepared.commit(&mapping.map);
            Ok(address)
        }
        MemoryCall::Deallocate { addr, .. } => {
            if !Space::user_range(addr.get(), size) {
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
            space.retry_invalidation()?;
            if map.is_live() {
                space.unmap_range(KVirt::wrap(addr.get()), size)?;
            }
            caller.pend.creations.forget(&map);
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
            if flags & !14 != 0 || flags & 2 == 0 || flags & 12 == 12 {
                return Err(MemoryFail::Denied);
            }
            let access = PteFlags::from_bits(flags).ok_or(MemoryFail::Denied)?;
            let source = if pole == env::PieToken::NONE {
                None
            } else {
                let caller = caller.as_ref().ok_or(MemoryFail::Denied)?;
                let pie = pend::locate(caller, pole).ok_or(MemoryFail::Denied)?;
                let meta = pie.pole().ok_or(MemoryFail::Denied)?;
                Some((pie, meta))
            };
            let _operation = match &source {
                Some((_, meta)) => Some(meta.operation().ok_or(MemoryFail::Busy)?),
                None => None,
            };
            let mut private = false;
            let mut ceiling = access;
            if let Some((pie, meta)) = &source {
                if !pie.alive() || !meta.owned() || meta.reserved() != 0 {
                    return Err(MemoryFail::Denied);
                }
                let permission = pie.permission();
                if !permission.contains(Permission::FETCH)
                    || access.contains(PteFlags::W) && !permission.contains(Permission::STORE)
                    || team_id.get() != 0 && !permission.contains(Permission::VEST)
                {
                    return Err(MemoryFail::Denied);
                }
                super::pie::usable::<env::PieFail>(pie).map_err(|_| MemoryFail::Denied)?;
                if offset % PAGE_SIZE != 0
                    || offset.checked_add(size).is_none_or(|end| end > meta.size())
                {
                    return Err(MemoryFail::NotAligned);
                }
                if team_id.get() == 0 {
                    if access.contains(PteFlags::X) {
                        return Err(MemoryFail::Denied);
                    }
                    ceiling = PteFlags::R
                        | if permission.contains(Permission::STORE) {
                            PteFlags::W
                        } else {
                            PteFlags::empty()
                        };
                } else if permission.contains(Permission::ONLY) {
                    if pie.sire().is_some()
                        || pie.heir().is_some()
                        || meta.owner() != ident.id
                        || !meta.exclusive()
                        || offset != 0
                        || size != meta.size()
                        || access.contains(PteFlags::X)
                        || meta.mapped()
                    {
                        return Err(MemoryFail::Busy);
                    }
                    target
                        .staged()
                        .try_reserve(1)
                        .map_err(|_| MemoryFail::OoM)?;
                    private = true;
                } else if !meta.readonly() {
                    return Err(MemoryFail::Busy);
                }
                if team_id.get() == 0 || private {
                    meta.reserve_record().map_err(|_| MemoryFail::OoM)?;
                }
            } else if offset != 0 || access.contains(PteFlags::X) {
                return Err(MemoryFail::Denied);
            }
            if at.get() != 0 && (at.get() % PAGE_SIZE != 0 || !Space::user_range(at.get(), size)) {
                return Err(MemoryFail::NotAligned);
            }
            space.retry_invalidation()?;
            let reservation = if at.get() == 0 {
                space.allocate_low(RegionId::Normal, size)?
            } else {
                space.reserve(RegionId::Normal, KVirt::wrap(at.get()), size)?
            };
            let current = access | PteFlags::A | PteFlags::D;
            let mapping = match &source {
                Some((_, meta)) => space.derive(
                    reservation,
                    &meta.canonical_map().ok_or(MemoryFail::Denied)?,
                    offset,
                    ceiling,
                    current,
                )?,
                None => space.map_lazy(reservation, ceiling, current)?,
            };
            let address = mapping.reservation.start().as_usize();
            if let Some((pie, _)) = &source {
                let permission = pie.permission();
                if !pie.alive()
                    || !permission.contains(Permission::FETCH)
                    || access.contains(PteFlags::W) && !permission.contains(Permission::STORE)
                {
                    let _ = space.dispose(mapping);
                    return Err(MemoryFail::Denied);
                }
            }
            if access.contains(PteFlags::X) {
                if let Err(error) = crate::memory::manager::sync_instructions() {
                    let _ = space.dispose(mapping);
                    return Err(error.into());
                }
            }
            if let Some((pie, meta)) = &source {
                if private {
                    let staged_map = mapping.map.clone();
                    if let Err(mapping) =
                        meta.record(pie.token(), space, mapping, Some(target.id), false)
                    {
                        let _ = space.dispose(mapping);
                        return Err(MemoryFail::Denied);
                    }
                    target.staged().push(crate::work::unit::team::Staging {
                        token: pie.token(),
                        meta: meta.clone(),
                        map: staged_map,
                    });
                    pend::notify(ident.id, pole);
                } else if team_id.get() == 0 {
                    if let Err(mapping) = meta.record(pie.token(), space, mapping, None, false) {
                        let _ = space.dispose(mapping);
                        return Err(MemoryFail::Denied);
                    }
                }
            }
            Ok(address)
        }
        MemoryCall::Munmap { addr, .. } => {
            if team_id.get() == 0 {
                let ranges = space.tree.ranges()?;
                if let Some(caller) = &caller {
                    let pies = caller.pend.pies.lock();
                    if ranges
                        .iter()
                        .filter(|(_, range)| {
                            range.start.as_usize() < addr.get().saturating_add(size)
                                && range.end() >= addr.get()
                        })
                        .any(|(map, _)| {
                            pies.iter().any(|pie| {
                                pie.snapshot().pole().is_some_and(|meta| meta.opened(map))
                            })
                        })
                    {
                        return Err(MemoryFail::Denied);
                    }
                }
            }
            let staged = if target.staged_len() == 0 {
                None
            } else {
                target
                    .staged()
                    .iter()
                    .position(|item| item.map.initial_range() == (KVirt::wrap(addr.get()), size))
            };
            if let Some(index) = staged {
                let (meta, token) = {
                    let items = target.staged();
                    (items[index].meta.clone(), items[index].token)
                };
                let _operation = meta.operation().ok_or(MemoryFail::Busy)?;
                space.unmap_range(KVirt::wrap(addr.get()), size)?;
                meta.detach_staging(token, target.id);
                target.staged().remove(index);
                pend::notify(meta.owner(), token);
                return Ok(0);
            }
            if target.staged_len() != 0
                && target.staged().iter().any(|item| {
                    let (start, len) = item.map.initial_range();
                    addr.get() < start.as_usize() + len
                        && start.as_usize() < addr.get().saturating_add(size)
                })
            {
                return Err(MemoryFail::Busy);
            }
            space.unmap_range(KVirt::wrap(addr.get()), size)?;
            Ok(0)
        }
        MemoryCall::Mprotect { addr, flags, .. } => {
            if !Space::user_range(addr.get(), size) || addr.get() % PAGE_SIZE != 0 {
                return Err(MemoryFail::NotAligned);
            }
            if flags & !14 != 0 || flags & 14 == 0 || flags & 4 != 0 && flags & 2 == 0 {
                return Err(MemoryFail::Denied);
            }
            let access = PteFlags::from_bits(flags).ok_or(MemoryFail::Denied)?;
            space.retry_invalidation()?;
            let end = addr.get() + size;
            let ranges = space.tree.ranges()?;
            let mut updates = Vec::new();
            updates
                .try_reserve(ranges.len())
                .map_err(|_| MemoryFail::OoM)?;
            let mut covered = addr.get();
            for (map, range) in ranges {
                if range.start.as_usize() >= end || range.end() < addr.get() {
                    continue;
                }
                let lo = range.start.as_usize().max(addr.get());
                let hi = (range.end() + 1).min(end);
                if lo != covered {
                    return Err(MemoryFail::NoRegion);
                }
                if !range.ceiling.contains(access) {
                    return Err(MemoryFail::WidenDenied);
                }
                updates.push((map, KVirt::wrap(lo), hi - lo));
                covered = hi;
            }
            if covered != end {
                return Err(MemoryFail::NoRegion);
            }
            for (map, start, size) in updates {
                space.tree.protect(&map, start, size, access)?;
            }
            if access.contains(PteFlags::X) {
                crate::memory::manager::sync_instructions()?;
            }
            Ok(0)
        }
    }
}
