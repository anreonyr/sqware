use alloc::vec::Vec;
use env::{FlatReference, PieToken, TaskId};

use super::memory::Memory;
use super::{Close, DeviceRange, Failure, MemoryEffects, Phase, Registration, Scope};

pub struct Holds {
    scopes: Vec<Scope>,
    next: usize,
    closed: bool,
}

impl Holds {
    pub const fn new() -> Self {
        Self {
            scopes: Vec::new(),
            next: 1,
            closed: false,
        }
    }

    pub fn attach(&mut self, scope: Scope) -> Result<(), Failure> {
        if self.closed {
            return Err(Failure::Closed);
        }
        if self
            .scopes
            .iter()
            .any(|other| other.range.overlaps(scope.range))
        {
            return Err(Failure::Conflict);
        }
        if scope
            .platform
            .as_ref()
            .is_some_and(|platform| platform.range() != scope.range)
        {
            return Err(Failure::Denied);
        }
        self.scopes.try_reserve(1).map_err(|_| Failure::Full)?;
        self.scopes.push(scope);
        Ok(())
    }

    /// Reserve metadata before Acquire; after success the reference has exactly one owner.
    pub fn register(
        &mut self,
        registration: Registration,
        effects: &mut impl MemoryEffects,
    ) -> Result<usize, Failure> {
        let Registration {
            range,
            source,
            retention,
        } = registration;
        if self.closed {
            return Err(Failure::Closed);
        }
        if retention == PieToken::NONE {
            return Err(Failure::Denied);
        }
        let next = self.next.checked_add(1).ok_or(Failure::Full)?;
        let scope = self
            .scopes
            .iter_mut()
            .find(|scope| scope.range == range)
            .ok_or(Failure::Unknown)?;
        if scope.phase != Phase::Open {
            return Err(Failure::Closed);
        }
        scope.memory.try_reserve(1).map_err(|_| Failure::Full)?;
        let reference = effects.acquire(source, retention)?;
        let id = self.next;
        scope.memory.push(Memory {
            id,
            reference,
            published: false,
        });
        self.next = next;
        Ok(id)
    }

    /// Mark reachable before executing publication: even a partial failure needs isolation.
    /// The callback must not publish any other memory or retain a path around this gate.
    ///
    /// 同步那一档：宿主夹具与将来一次做完的调用方用它；产品那一侧要跨帧，
    /// 走 [`Holds::publish_begin`] + [`Holds::publish_end`]。
    #[allow(
        dead_code,
        reason = "Synchronous publication; the frame-driven manager uses publish_begin."
    )]
    pub fn publish(
        &mut self,
        id: usize,
        publish: impl FnOnce(FlatReference) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        let reference = self.publish_begin(id)?;
        let result = publish(reference);
        self.publish_end(id, result)
    }

    /// 发布门的第一步：**先把可达标记点上**，再把引用交出来。
    ///
    /// 返回调用方自己执行发布（铺表、立队列、编程 `DDTP`），这件事可以**跨帧**做完；
    /// 做完无论成败都回用 [`Holds::publish_end`] 交结果。标记在**执行发布之前**点上，
    /// 故部分失败仍会被隔离。
    pub fn publish_begin(&mut self, id: usize) -> Result<FlatReference, Failure> {
        if self.closed {
            return Err(Failure::Closed);
        }
        let scope = self
            .scopes
            .iter_mut()
            .find(|scope| scope.memory.iter().any(|memory| memory.id == id))
            .ok_or(Failure::Unknown)?;
        if scope.phase != Phase::Open {
            return Err(Failure::Closed);
        }
        if scope.platform.is_none() {
            return Err(Failure::Unavailable);
        }
        let memory = scope
            .memory
            .iter_mut()
            .find(|memory| memory.id == id)
            .ok_or(Failure::Unknown)?;
        if memory.published {
            return Err(Failure::Conflict);
        }
        memory.published = true;
        Ok(memory.reference)
    }

    /// 发布门的结果：失败就进 `Closing(Fault)`（这一档仍会走平台停机），成功不动状态。
    pub fn publish_end(&mut self, id: usize, result: Result<(), Failure>) -> Result<(), Failure> {
        if let Err(failure) = result {
            let scope = self
                .scopes
                .iter_mut()
                .find(|scope| scope.memory.iter().any(|memory| memory.id == id))
                .ok_or(Failure::Unknown)?;
            scope.phase = Phase::Closing(Close::Fault);
            scope.failure = Some(failure);
        }
        result
    }

    pub fn close(&mut self, range: DeviceRange, reason: Close) -> Result<(), Failure> {
        let scope = self
            .scopes
            .iter_mut()
            .find(|scope| scope.range == range)
            .ok_or(Failure::Unknown)?;
        Self::begin_close(scope, reason);
        Ok(())
    }

    /// Death closes publication immediately, without waiting for ordinary task cleanup.
    pub fn offline(&mut self, controller: TaskId) {
        for scope in &mut self.scopes {
            if scope.controller == controller {
                Self::begin_close(scope, Close::Fault);
            }
        }
    }

    pub fn close_all(&mut self) {
        self.closed = true;
        for scope in &mut self.scopes {
            Self::begin_close(scope, Close::Normal);
        }
    }

    fn begin_close(scope: &mut Scope, reason: Close) {
        match (scope.phase, reason) {
            (Phase::Open, _) | (Phase::Closing(Close::Normal), Close::Fault) => {
                scope.phase = Phase::Closing(reason);
            }
            _ => {}
        }
    }

    /// No table lock or failed-service RPC is held across platform progress.
    /// Failure in one scope does not prevent independent scopes from settling.
    pub fn retry(&mut self, effects: &mut impl MemoryEffects) -> bool {
        let mut failed = false;
        for scope in &mut self.scopes {
            let previous = scope.failure;
            Self::progress(scope, effects);
            failed |= scope.failure.is_some() && scope.failure != previous;
        }
        failed
    }

    fn progress(scope: &mut Scope, effects: &mut impl MemoryEffects) {
        let Phase::Closing(reason) = scope.phase else {
            if scope.phase == Phase::Stopped {
                Self::release(scope, effects);
            }
            return;
        };
        if scope.memory.iter().any(|memory| memory.published) {
            let Some(platform) = scope.platform.as_mut() else {
                scope.failure = Some(Failure::Unavailable);
                return;
            };
            let result = match reason {
                Close::Normal => platform.synchronize(),
                Close::Fault => platform.isolate(),
            };
            if let Err(failure) = result {
                scope.failure = Some(failure);
                return;
            }
        }
        scope.phase = Phase::Stopped;
        Self::release(scope, effects);
    }

    fn release(scope: &mut Scope, effects: &mut impl MemoryEffects) {
        scope.failure = None;
        let mut at = 0;
        while at < scope.memory.len() {
            match effects.release(scope.memory[at].reference) {
                Ok(()) => {
                    scope.memory.remove(at);
                }
                Err(failure) => {
                    scope.failure = Some(failure);
                    at += 1;
                }
            }
        }
    }

    pub fn controllers(&self) -> impl Iterator<Item = TaskId> + '_ {
        self.scopes
            .iter()
            .filter(|scope| scope.phase == Phase::Open)
            .map(|scope| scope.controller)
    }

    /// 这一段此刻还持着的引用（放下之后是 `None`）。
    ///
    /// 调用方（本驱动）用它读真实页起址与算“还持有几页”；不产生第二份引用账。
    pub fn reference(&self, range: DeviceRange) -> Option<FlatReference> {
        self.scopes
            .iter()
            .find(|scope| scope.range == range)
            .and_then(|scope| scope.memory.first().map(|memory| memory.reference))
    }

    /// 这一段此刻的失败现场（协调入口据此报原始失败）。
    pub fn failure(&self, range: DeviceRange) -> Option<Failure> {
        self.scopes
            .iter()
            .find(|scope| scope.range == range)
            .and_then(|scope| scope.failure)
    }

    /// 把这一段从账上退掉（**只在它不持任何引用时**）。
    ///
    /// 给“登记之后又没落成”的那一条用：引用已由 `retry` 放下、或者从未取得，
    /// 调用方不必为一次没落地的尝试留一个孤儿 scope。
    pub fn forget(&mut self, range: DeviceRange) -> bool {
        let Some(at) = self.scopes.iter().position(|scope| scope.range == range) else {
            return false;
        };
        if !self.scopes[at].memory.is_empty() {
            return false;
        }
        self.scopes.remove(at);
        true
    }

    pub fn occupied(
        &self,
    ) -> impl Iterator<Item = (DeviceRange, TaskId, Phase, usize, Option<Failure>)> + '_ {
        self.scopes
            .iter()
            .filter(|scope| !scope.memory.is_empty())
            .map(|scope| {
                (
                    scope.range,
                    scope.controller,
                    scope.phase,
                    scope.memory.len(),
                    scope.failure,
                )
            })
    }
}
