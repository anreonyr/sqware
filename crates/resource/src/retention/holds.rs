use alloc::vec::Vec;
use env::{FlatReference, PieToken, TaskId};

use super::exposure::Exposure;
use super::memory::Memory;
use super::{
    Close, Failure, MemoryEffects, Occupancy, Phase, Registration, RetentionId, Scope, ScopeId,
};

pub struct Holds {
    scopes: Vec<(ScopeId, Scope)>,
    next_scope: usize,
    next_retention: usize,
    closed: bool,
}

impl Holds {
    pub const fn new() -> Self {
        Self {
            scopes: Vec::new(),
            next_scope: 1,
            next_retention: 1,
            closed: false,
        }
    }

    pub fn attach(&mut self, scope: Scope) -> Result<ScopeId, Failure> {
        if self.closed {
            return Err(Failure::Closed);
        }
        if self.scopes.iter().any(|(_, other)| {
            other.phase != Phase::Released
                && other.domain_range.domain == scope.domain_range.domain
                && other.domain_range.range.overlaps(scope.domain_range.range)
        }) {
            return Err(Failure::Conflict);
        }
        if scope
            .platform
            .as_ref()
            .is_some_and(|platform| platform.domain_range() != scope.domain_range)
        {
            return Err(Failure::Denied);
        }
        let next = self.next_scope.checked_add(1).ok_or(Failure::Full)?;
        self.scopes.try_reserve(1).map_err(|_| Failure::Full)?;
        let id = ScopeId::mint(self.next_scope);
        self.scopes.push((id, scope));
        self.next_scope = next;
        Ok(id)
    }

    /// 先占下元数据再取引用；成功之后这一份引用只有一个主人。
    pub fn register(
        &mut self,
        registration: Registration,
        effects: &mut impl MemoryEffects,
    ) -> Result<RetentionId, Failure> {
        let Registration {
            scope: scope_id,
            source,
            retention,
        } = registration;
        if self.closed {
            return Err(Failure::Closed);
        }
        if retention == PieToken::NONE {
            return Err(Failure::Denied);
        }
        let next = self.next_retention.checked_add(1).ok_or(Failure::Full)?;
        let Some((_, scope)) = self.scopes.iter_mut().find(|(id, _)| *id == scope_id) else {
            return Err(Failure::Unknown);
        };
        if scope.phase != Phase::Open {
            return Err(Failure::Closed);
        }
        scope.memory.try_reserve(1).map_err(|_| Failure::Full)?;
        let reference = effects.acquire(source, retention)?;
        let id = RetentionId::mint(self.next_retention);
        scope.memory.push(Memory {
            id,
            reference,
            exposure: Exposure::Held,
        });
        self.next_retention = next;
        Ok(id)
    }

    /// 一次做完发布：先点上可达标记，再执行 `publish`。
    #[allow(
        dead_code,
        reason = "Synchronous publication; the frame-driven manager uses publish_begin."
    )]
    pub fn publish(
        &mut self,
        retention: RetentionId,
        publish: impl FnOnce(FlatReference) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        let reference = self.publish_begin(retention)?;
        let result = publish(reference);
        self.publish_end(retention, result)
    }

    /// 先把可达标记点上，再把引用交出来；标记不可逆。
    pub fn publish_begin(&mut self, retention: RetentionId) -> Result<FlatReference, Failure> {
        if self.closed {
            return Err(Failure::Closed);
        }
        let Some((_, scope)) = self
            .scopes
            .iter_mut()
            .find(|(_, scope)| scope.memory.iter().any(|memory| memory.id == retention))
        else {
            return Err(Failure::Unknown);
        };
        if scope.phase != Phase::Open {
            return Err(Failure::Closed);
        }
        if scope.platform.is_none() {
            return Err(Failure::Unavailable);
        }
        let Some(memory) = scope
            .memory
            .iter_mut()
            .find(|memory| memory.id == retention)
        else {
            return Err(Failure::Unknown);
        };
        if memory.exposure == Exposure::Reachable {
            return Err(Failure::Conflict);
        }
        memory.exposure = Exposure::Reachable;
        Ok(memory.reference)
    }

    /// 发布门的收尾：`Err` 只作用于已点上可达标记且仍 `Open` 的那一份。
    pub fn publish_end(
        &mut self,
        retention: RetentionId,
        result: Result<(), Failure>,
    ) -> Result<(), Failure> {
        if let Err(failure) = result {
            let reachable = self.scopes.iter_mut().find(|(_, scope)| {
                scope.phase == Phase::Open
                    && scope.memory.iter().any(|memory| {
                        memory.id == retention && memory.exposure == Exposure::Reachable
                    })
            });
            if let Some((_, scope)) = reachable {
                scope.phase = Phase::Stopping(Close::Fault);
                scope.failure = Some(failure);
            }
        }
        result
    }

    pub fn close(&mut self, scope: ScopeId, reason: Close) -> Result<(), Failure> {
        let Some((_, held)) = self.scopes.iter_mut().find(|(id, _)| *id == scope) else {
            return Err(Failure::Unknown);
        };
        Self::begin_close(held, reason);
        Ok(())
    }

    /// 控制者死亡按最严档关掉它名下每一格，不等普通清理。
    pub fn offline(&mut self, controller: TaskId) {
        for (_, scope) in &mut self.scopes {
            if scope.controller == controller {
                Self::begin_close(scope, Close::Fault);
            }
        }
    }

    pub fn close_all(&mut self) {
        self.closed = true;
        for (_, scope) in &mut self.scopes {
            Self::begin_close(scope, Close::Normal);
        }
    }

    fn begin_close(scope: &mut Scope, reason: Close) {
        match (scope.phase, reason) {
            (Phase::Open, _) | (Phase::Stopping(Close::Normal), Close::Fault) => {
                scope.phase = Phase::Stopping(reason);
            }
            _ => {}
        }
    }

    /// 每一格推一步；返回本轮是否出现新失败。
    pub fn retry(&mut self, effects: &mut impl MemoryEffects) -> bool {
        let mut failed = false;
        for (_, scope) in &mut self.scopes {
            let previous = scope.failure;
            Self::progress(scope, effects);
            failed |= scope.failure.is_some() && scope.failure != previous;
        }
        failed
    }

    fn progress(scope: &mut Scope, effects: &mut impl MemoryEffects) {
        let Phase::Stopping(reason) = scope.phase else {
            if scope.phase == Phase::Stopped {
                Self::release(scope, effects);
            }
            return;
        };
        if scope
            .memory
            .iter()
            .any(|memory| memory.exposure == Exposure::Reachable)
        {
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
        // 引用清空 = 这一格释放；平台随之放下。
        if scope.memory.is_empty() {
            scope.phase = Phase::Released;
            scope.platform = None;
        }
    }

    pub fn controllers(&self) -> impl Iterator<Item = TaskId> + '_ {
        self.scopes
            .iter()
            .filter(|(_, scope)| scope.phase == Phase::Open)
            .map(|(_, scope)| scope.controller)
    }

    pub fn occupied(&self) -> impl Iterator<Item = Occupancy> + '_ {
        self.scopes
            .iter()
            .filter(|(_, scope)| scope.phase != Phase::Released)
            .map(|(id, scope)| Occupancy {
                scope: *id,
                domain_range: scope.domain_range,
                controller: scope.controller,
                phase: scope.phase,
                references: scope.memory.len(),
                failure: scope.failure,
            })
    }
}
