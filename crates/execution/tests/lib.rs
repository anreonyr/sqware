#![cfg(test)]

mod memory {
    pub const PAGE_SIZE: usize = env::PAGE_SIZE;

    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    pub static CALLS: AtomicUsize = AtomicUsize::new(0);
    pub static BYTES: AtomicUsize = AtomicUsize::new(0);
    pub static FAIL: AtomicBool = AtomicBool::new(false);
    /// 归还注入：让一次 `detach` / `release` 真失败，测失败分支。
    pub static DETACH_FAIL: AtomicBool = AtomicBool::new(false);
    pub static RELEASE_FAIL: AtomicBool = AtomicBool::new(false);
    pub static DETACHED: AtomicUsize = AtomicUsize::new(0);
    pub static RELEASED: AtomicUsize = AtomicUsize::new(0);
    static NEXT_TASK: AtomicUsize = AtomicUsize::new(128);

    std::thread_local! {
        static TASK: Cell<usize> = Cell::new(NEXT_TASK.fetch_add(1, Ordering::Relaxed));
    }

    pub fn current_task() -> env::TaskId {
        env::TaskId::new(TASK.get())
    }

    pub fn set_task(task: usize) {
        TASK.set(task);
    }

    /// 一页区段的存账：号 → (地址, 长度)。归还按号查，不靠地址猜。
    static SEGMENTS: Mutex<Vec<(usize, usize, usize)>> = Mutex::new(Vec::new());
    static REFERENCES: Mutex<Vec<(usize, env::TaskId)>> = Mutex::new(Vec::new());
    static NEXT: AtomicUsize = AtomicUsize::new(1);

    pub fn references() -> usize {
        REFERENCES.lock().unwrap().len()
    }

    pub fn allocate_shared_raw(
        size: usize,
    ) -> Result<(usize, env::ViewId, env::FlatReference), ()> {
        assert_eq!(size % PAGE_SIZE, 0);
        if FAIL.load(Ordering::Relaxed) {
            return Err(());
        }
        let layout = Layout::from_size_align(size, PAGE_SIZE).unwrap();
        // SAFETY: the page layout is valid and System is independent of Talc.
        let ptr = unsafe { System.alloc(layout) };
        if ptr.is_null() {
            return Err(());
        }
        CALLS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(size, Ordering::Relaxed);
        let raw = NEXT.fetch_add(1, Ordering::Relaxed);
        SEGMENTS.lock().unwrap().push((raw, ptr as usize, size));
        REFERENCES.lock().unwrap().push((raw, current_task()));
        Ok((
            ptr as usize,
            env::ViewId::from_bytes(&raw.to_le_bytes()).unwrap(),
            env::FlatReference::from_bytes(&raw.to_le_bytes()).unwrap(),
        ))
    }

    /// 解开一份来源引用：来源本身由关系保着，这里没有要释放的区段。
    pub fn release(reference: env::FlatReference) -> Result<(), ()> {
        RELEASED.fetch_add(1, Ordering::Relaxed);
        if RELEASE_FAIL.load(Ordering::Relaxed) {
            return Err(());
        }
        let mut references = REFERENCES.lock().unwrap();
        let index = references
            .iter()
            .position(|(raw, owner)| *raw == reference.get() && *owner == current_task())
            .ok_or(())?;
        references.remove(index);
        Ok(())
    }

    pub fn detach(_: env::TeamId, view: env::ViewId, _: env::ViewSelection) -> Result<(), ()> {
        DETACHED.fetch_add(1, Ordering::Relaxed);
        if DETACH_FAIL.load(Ordering::Relaxed) {
            return Err(());
        }
        let raw = view.get();
        let entry = {
            let mut segments = SEGMENTS.lock().unwrap();
            let at = segments.iter().position(|(held, _, _)| *held == raw);
            at.map(|at| segments.remove(at))
        };
        let Some((_, address, size)) = entry else {
            return Err(());
        };
        let layout = Layout::from_size_align(size, PAGE_SIZE).unwrap();
        // SAFETY: the segment is the original region Talc returned.
        unsafe { System.dealloc(address as *mut u8, layout) };
        BYTES.fetch_sub(size, Ordering::Relaxed);
        Ok(())
    }
}

#[path = "fixtures/state.rs"]
mod fixture_state;

#[path = "fixtures/kernel.rs"]
mod fixture_kernel;

#[path = "mechanism/mod.rs"]
mod mechanism;

#[path = "../src/memory/allocator.rs"]
mod allocator;

#[path = "../src/boot/args.rs"]
mod args;

#[test]
fn startup_arguments_preserve_pointer_and_count_order() {
    let values = [0x11usize, 0x22, 0x33];
    args::save_args(values.as_ptr() as usize, values.len());
    assert_eq!(args::args(), values);
    args::save_args(0, 0);
    assert!(args::args().is_empty());
}
