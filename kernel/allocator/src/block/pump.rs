use core::ptr::NonNull;

pub(super) struct Pump {
    head: Option<NonNull<u8>>,
}

// SAFETY: 裸指针只在持有泵锁时读写。
unsafe impl Send for Pump {}

impl Pump {
    pub(super) const fn new() -> Self {
        Self { head: None }
    }

    pub(super) unsafe fn push(&mut self, ptr: NonNull<u8>) {
        unsafe {
            ptr.cast::<Option<NonNull<u8>>>().write(self.head);
            self.head = Some(ptr);
        }
    }

    pub(super) fn take(&mut self) -> Option<NonNull<u8>> {
        self.head.take()
    }
}
