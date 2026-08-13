// LazyLock — 惰性初始化容器
//
// 首次访问（Deref 或 force）时通过初始化函数计算值并缓存，此后直接返回引用。
// 基于 OnceLock 实现，读取路径在初始化后无锁。
//
// const fn new 限制：初始化器类型为 `fn() -> T`（函数指针可在 const 上下文构造；
// 闭包不能）。需要捕获环境的闭包场景请改用 OnceLock + 运行时 get_or_init。

use core::ops::Deref;

use super::once::OnceLock;

/// 惰性初始化容器，首次访问时用 `init` 函数生成值。
#[allow(dead_code)] // 惰性初始化工具预留
pub struct LazyLock<T> {
    once: OnceLock<T>,
    // 初始化函数指针，仅在首次访问时调用一次
    init: fn() -> T,
}

// SAFETY: OnceLock<T> 已保证 T: Send + Sync 时的跨 hart 安全；
// init 为函数指针（Send + Sync）。
unsafe impl<T: Send + Sync> Sync for LazyLock<T> {}

#[allow(dead_code)]
impl<T> LazyLock<T> {
    /// 创建一个惰性容器，`init` 在首次访问时调用一次。
    pub const fn new(init: fn() -> T) -> Self {
        LazyLock {
            once: OnceLock::new(),
            init,
        }
    }

    /// 强制求值并返回引用；已初始化则直接返回缓存值。
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
