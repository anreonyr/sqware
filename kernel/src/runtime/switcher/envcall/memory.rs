use alloc::sync::Arc;

use env::{MemoryCall, MemoryFail};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::unit::space::Space;
use crate::work::unit::space::window::HeapWindow;
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
            MapError::NotMapped | MapError::SegmentMismatch | MapError::DramOverlap => {
                MemoryFail::Denied
            }
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
    use crate::memory::manager::entry::PteFlags;
    use crate::work::room::scheduler::core::current;
    use crate::work::unit::{
        gate::{self, Permission},
        space::{Pending, SegmentKind},
    };
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
    } else if raw_size == 0 || !raw_size.is_multiple_of(PAGE_SIZE) {
        return Err(MemoryFail::NotAligned);
    } else {
        raw_size
    };
    let team_id = match call {
        MemoryCall::Mmap { team, .. }
        | MemoryCall::Munmap { team, .. }
        | MemoryCall::Mprotect { team, .. } => team,
        _ => env::TeamId::new(0),
    };
    let caller = current().running_task();
    let target = if team_id.get() == 0 {
        ident.team.clone()
    } else {
        caller
            .as_ref()
            .and_then(|task| task.heir(team_id))
            .ok_or(MemoryFail::Denied)?
    };
    let _construction = if team_id.get() != 0 {
        Some(target.operation().ok_or(MemoryFail::Busy)?)
    } else {
        None
    };
    if team_id.get() != 0 && target.ready() {
        return Err(MemoryFail::Denied);
    }
    let space = &target.space;
    match call {
        MemoryCall::Allocate { .. } => HeapWindow::allocate(space, size)
            .map(|span| span.va.as_usize())
            .map_err(Into::into),
        MemoryCall::Deallocate { addr, .. } => {
            if Space::user_range(addr.get(), size)
                && HeapWindow::deallocate(space, KVirt::wrap(addr.get()), size)
            {
                Ok(0)
            } else {
                Err(MemoryFail::Denied)
            }
        }
        MemoryCall::Mmap {
            at,
            backing,
            offset,
            flags,
            ..
        } => {
            if flags & !14 != 0 || flags & 2 == 0 || flags & 12 == 12 {
                return Err(MemoryFail::Denied);
            }
            let access = PteFlags::from_bits(flags).ok_or(MemoryFail::Denied)?;
            let pte = space.pte_policy(access | PteFlags::V | PteFlags::A | PteFlags::D);
            let mut source = None;
            let mut operation = None;
            let mut private = false;
            let mut ceiling = access;
            if backing != env::PieToken::NONE {
                let caller = caller.as_ref().ok_or(MemoryFail::Denied)?;
                let pie = gate::locate(caller, backing).ok_or(MemoryFail::Denied)?;
                let p = pie.pole().ok_or(MemoryFail::Denied)?;
                operation = Some(p.backing().operation().ok_or(MemoryFail::Busy)?);
                if !p.alive()
                    || p.backing().reserved() != 0
                    || !p.backing().owned()
                {
                    return Err(MemoryFail::Denied);
                }
                let permission = pie.permission();
                if !permission.contains(Permission::FETCH)
                    || access.contains(PteFlags::W) && !permission.contains(Permission::STORE)
                    || team_id.get() != 0 && !permission.contains(Permission::VEST)
                {
                    return Err(MemoryFail::Denied);
                }
                super::pie::usable::<env::PieFail>(&pie)
                    .map_err(|_| MemoryFail::Denied)?;
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
                    if target.ready() { return Err(MemoryFail::Denied); }
                    private = true;
                    if pie.sire().is_some()
                        || p.owner() != caller.ident.id
                        || !p.backing().exclusive()
                        || offset != 0
                        || size != p.backing().size()
                        || access.contains(PteFlags::X)
                        || !p.backing().unmapped()
                        || p.mapped()
                    {
                        return Err(MemoryFail::Busy);
                    }
                    target
                        .staged()
                        .try_reserve(1)
                        .map_err(|_| MemoryFail::OoM)?;
                    ceiling = access;
                } else {
                    if !p.backing().readonly() {
                        return Err(MemoryFail::Busy);
                    }
                    ceiling = access;
                }
                p
                    .backing()
                    .address(offset, size)
                    .map_err(MemoryFail::from)?;
                source = Some((pie, p));
            } else if offset != 0 || access.contains(PteFlags::X) {
                return Err(MemoryFail::Denied);
            }
            if at.get() != 0
                && (!at.get().is_multiple_of(PAGE_SIZE) || !Space::user_range(at.get(), size))
            {
                return Err(MemoryFail::NotAligned);
            }
            let va = space
                .with_flush(|inner| {
                    let va = if at.get() == 0 {
                        HeapWindow::locate(inner, size)?
                    } else {
                        let va = KVirt::wrap(at.get());
                        if inner.overlaps(va, size) {
                            return Err(MapError::AlreadyMapped);
                        }
                        va
                    };
                    inner.allocate(SegmentKind::Normal, va.as_usize(), size)?;
                    let result = if let Some((_, p)) = &source {
                        inner.backed(va, p.backing().clone(), offset, size, pte, ceiling)
                    } else {
                        inner.map(va, size, pte, Some(Pending::Lazy))
                    };
                    if let Err(error) = result {
                        inner.deallocate(SegmentKind::Normal, va.as_usize(), size);
                        return Err(error);
                    }
                    if let Some((pie, _)) = &source {
                        if team_id.get() == 0 || private {
                            inner.bind(va, pie.token());
                        }
                        if private {
                            inner.private(va);
                        }
                    } else {
                        inner.limit(va, access);
                    }
                    Ok(va)
                })
                .map_err(MemoryFail::from)?;
            if let Some((pie, p)) = source {
                let span = crate::work::unit::space::Span::new(SegmentKind::Normal, va, size, None);
                if private {
                    p.backing().reserve(target.id);
                    target.staged().push(crate::work::unit::team::Staging {
                        token: pie.token(),
                        meta: p.clone(),
                        span,
                    });
                } else if team_id.get() == 0 {
                    if let Err(error) = p.record(pie.token(), space, span) {
                        space.release(span).expect("map rollback");
                        return Err(match error {
                            env::PieFail::OoM => MemoryFail::OoM,
                            _ => MemoryFail::Denied,
                        });
                    }
                }
            }
            if access.contains(PteFlags::X) {
                if let Err(error) = crate::work::unit::space::sync_instructions() {
                    space
                        .release(crate::work::unit::space::Span::new(
                            SegmentKind::Normal,
                            va,
                            size,
                            None,
                        ))
                        .expect("instruction sync rollback");
                    return Err(error.into());
                }
            }
            drop(operation);
            if private { gate::notify(ident.id, backing); }
            Ok(va.as_usize())
        }
        MemoryCall::Munmap { addr, .. } => {
            let item = if target.staged_len() == 0 { None } else {
                target.staged().iter().position(|item| {
                    item.span.va.as_usize() == addr.get() && item.span.size.get() == size
                })
            };
            if let Some(index) = item {
                let (meta, span, token) = {
                    let items = target.staged();
                    (items[index].meta.clone(), items[index].span, items[index].token)
                };
                let _operation = meta.backing().operation().ok_or(MemoryFail::Busy)?;
                space.release(span).map_err(MemoryFail::from)?;
                meta.backing().unreserve();
                target.staged().remove(index);
                drop(_operation);
                gate::notify(meta.owner(), token);
                return Ok(0);
            }
            if target.staged_len() != 0 && target.staged().iter().any(|item| {
                addr.get() < item.span.va.as_usize() + item.span.size.get()
                    && item.span.va.as_usize() < addr.get().saturating_add(size)
            }) {
                return Err(MemoryFail::Busy);
            }
            space
                .unmap_user(addr.get(), size)
                .map(|_| 0)
                .map_err(Into::into)
        }
        MemoryCall::Mprotect { addr, flags, .. } => space
            .protect_user(addr.get(), size, flags)
            .map(|_| 0)
            .map_err(Into::into),
    }
}
