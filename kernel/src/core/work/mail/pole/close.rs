use alloc::sync::Arc;
use alloc::vec::Vec;

use env::PieFail;

use crate::core::work::pend::SourceAuthorization;
use crate::core::work::unit::space::ViewBinding;

use super::{PoleMeta, PoleOperation, pie_map_error, seal};

pub(crate) fn pending_close(meta: &PoleMeta) -> bool {
    meta.has_maps()
        || meta
            .authorization()
            .is_some_and(|source| source.pending_close())
}

/// 调用者持有根资源的操作租约；CPU 关闭不在全局提交锁下等待。
pub(crate) fn close(root: &Arc<PoleMeta>) -> Result<(), PieFail> {
    let Some(source) = root.authorization() else {
        return root.close_maps(None);
    };
    let mut pending = Vec::new();
    pending.try_reserve(1).map_err(|_| PieFail::OoM)?;
    pending.push(source);
    while let Some(source) = pending.pop() {
        let meta = source.root().map(|(meta, _, _)| meta);
        let _operation = match &meta {
            Some(meta) if !Arc::ptr_eq(meta, root) => {
                Some(PoleOperation::acquire(meta).ok_or(PieFail::Busy)?)
            }
            _ => None,
        };
        if meta.as_ref().is_some_and(|meta| meta.reserved() != 0) {
            return Err(PieFail::Busy);
        }
        let bindings = {
            let _commit = crate::core::work::commit();
            let bindings = source.bindings()?;
            source.revoke()?;
            if let Some(meta) = &meta {
                seal(meta);
            }
            bindings
        };
        if let Some(meta) = &meta {
            meta.close_maps(None)?;
        }
        close_bindings(&source, bindings)?;
        let _commit = crate::core::work::commit();
        source.reap_derivations();
        let children = source.children()?;
        pending
            .try_reserve(children.len())
            .map_err(|_| PieFail::OoM)?;
        pending.extend(children);
        source.settle();
    }
    Ok(())
}

fn close_bindings(
    source: &Arc<SourceAuthorization>,
    bindings: Vec<Arc<ViewBinding>>,
) -> Result<(), PieFail> {
    for binding in bindings {
        let (space, view) = {
            let _commit = crate::core::work::commit();
            if !source.owns_binding(&binding) {
                continue;
            }
            let space = binding.space().upgrade().ok_or(PieFail::Busy)?;
            let entry = space.views.registered(binding.id()).ok_or(PieFail::Busy)?;
            if !Arc::ptr_eq(&entry.source, source) || !Arc::ptr_eq(&entry.binding, &binding) {
                return Err(PieFail::Busy);
            }
            (space, entry.view().cloned())
        };
        let id = binding.id();
        if !space.nest.settled(id)
            && let Err(error) = space.nest.retry_invalidation()
            && !space.nest.settled(id)
        {
            return Err(pie_map_error(error));
        }
        if !space.nest.settled(id) {
            let view = view.ok_or(PieFail::Busy)?;
            if let Err(error) = space.withdraw(view.whole())
                && !space.nest.settled(id)
            {
                return Err(pie_map_error(error));
            }
        }
        if !space.nest.settled(id) {
            return Err(PieFail::Busy);
        }
        space.retire_view(id);
    }
    Ok(())
}
