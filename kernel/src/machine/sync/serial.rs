use core::sync::atomic::{AtomicUsize, Ordering};

pub(crate) fn next_id(next: &AtomicUsize) -> Option<usize> {
    next.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
        (id != 0 && id <= isize::MAX as usize).then(|| id + 1)
    })
    .ok()
}
