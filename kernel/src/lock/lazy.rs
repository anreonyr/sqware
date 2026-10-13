use core::ops::Deref;

use super::once::OnceLock;

#[allow(dead_code)]
pub struct LazyLock<T> {
    once: OnceLock<T>,
    init: fn() -> T,
}

unsafe impl<T: Send + Sync> Sync for LazyLock<T> {}

#[allow(dead_code)]
impl<T> LazyLock<T> {
    pub const fn new(init: fn() -> T) -> Self {
        LazyLock {
            once: OnceLock::new(),
            init,
        }
    }

    pub fn force(&self) -> &T {
        self.once.get_or_init(self.init)
    }
}

impl<T> Deref for LazyLock<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.force()
    }
}
