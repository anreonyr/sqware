use std::sync::Mutex;

pub(crate) struct TestLock(Mutex<()>);

unsafe impl kernel_allocator::Lock for TestLock {
    type Guard<'a>
        = std::sync::MutexGuard<'a, ()>
    where
        Self: 'a;

    fn new(_role: Option<kernel_allocator::LockRole>) -> Self {
        Self(Mutex::new(()))
    }

    fn lock(&self) -> Self::Guard<'_> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}
