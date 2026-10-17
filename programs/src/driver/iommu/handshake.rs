//! 一次有界握手的期限：建立时定下，跨帧保留。

/// 一次有界握手的期限：建立时定下，**跨帧保留**。
///
/// 交出去之后可以反复问（[`Handshake::expired`]），期限不再变——这正是
/// 「一帧只读一次、没等到就留到下一帧」与「重复调用不重置时限」共同要的那一格。
#[derive(Clone, Copy)]
pub struct Handshake {
    until: u64,
}

impl Handshake {
    /// 从这一刻起给这次握手 `limit` 纳秒。
    pub fn new(limit: u64) -> Self {
        Self {
            until: env::chrono::clock().saturating_add(limit),
        }
    }

    /// 期限过了没有。
    pub fn expired(&self) -> bool {
        env::chrono::clock() >= self.until
    }

    /// 同步那一档：读到 `done` 成立或到期限为止，返最后那一刻的读数。
    /// 跨帧那一档不用它——那一路每帧只读一次，期限由 [`Handshake::expired`] 判。
    pub fn wait<T: Copy>(&self, mut read: impl FnMut() -> T, done: impl Fn(T) -> bool) -> T {
        let mut seen = read();
        while !done(seen) && !self.expired() {
            seen = read();
        }
        seen
    }
}
