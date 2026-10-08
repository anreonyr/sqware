//! 订阅状态、路径过滤与事件投递。
use ::resource::raw::{Hole, alive, reserve};
use alloc::vec::Vec;
use env::{PieToken, TaskId, Wait};
use system_api::operator::path::{Path, PathBuf};
use system_api::operator::{Event, WATCH_MARK};
use wire::Message;

/// 已核验的订阅端点；字段不能由接入方直接构造。
pub(crate) struct Subscription {
    who: TaskId,
    road: PathBuf,
    hole: PieToken,
}
impl Subscription {
    pub(crate) fn import(who: TaskId, road: PathBuf, hole: PieToken) -> Result<Self, ()> {
        if !alive(hole)
            || !matches!(reserve(hole), Ok((giver, owner, mark))
            if giver == who && owner == who && mark == WATCH_MARK)
        {
            return Err(());
        }
        Ok(Self { who, road, hole })
    }
}

struct Subscriber {
    who: TaskId,
    filter: PathBuf,
    hole: PieToken,
}

pub(crate) struct Watchers {
    list: Vec<Subscriber>,
    buffer: <Event as Message>::Buf,
    seq: u64,
}

impl Watchers {
    pub(crate) fn new() -> Self {
        Self {
            list: Vec::new(),
            buffer: Event::EMPTY,
            seq: 0,
        }
    }

    pub(crate) fn join(&mut self, subscription: Subscription) -> Result<(), ()> {
        let Subscription { who, road, hole } = subscription;
        self.list.retain(|subscriber| alive(subscriber.hole));
        if self
            .list
            .iter()
            .any(|one| one.who == who && one.filter.as_str() == road.as_str())
        {
            return Err(());
        }
        self.list.try_reserve(1).map_err(|_| ())?;
        let filter = road;
        self.list.push(Subscriber { who, filter, hole });
        let at = self.list.len() - 1;
        programs::debug::put(&alloc::format!(
            "operator: watch joined who={} road={} watchers={}",
            who.get(),
            self.list[at].filter,
            self.len()
        ));
        Ok(())
    }

    pub(crate) fn publish(&mut self, mut ev: Event) -> usize {
        self.seq += 1;
        ev.seq = self.seq;
        let Some(size) = ev.store(&mut self.buffer) else {
            programs::debug::put(&alloc::format!(
                "operator: watch event too long seq={}",
                ev.seq
            ));
            return 0;
        };
        let bytes = &self.buffer[..size];
        let road = ev.road.as_str();
        let mut sent = 0usize;
        let mut dead: Vec<usize> = Vec::new();
        for (at, one) in self.list.iter_mut().enumerate() {
            if !prefix_of(&one.filter, road) {
                continue;
            }
            match Hole::from_raw(one.hole).push(bytes, Wait::POLL) {
                Ok(()) => sent += 1,
                Err(e) if e.source.is_busy() => {}
                Err(_) => dead.push(at),
            }
        }
        for at in dead.into_iter().rev() {
            let one = self.list.swap_remove(at);
            programs::debug::put(&alloc::format!(
                "operator: watch dropped who={} watchers={}",
                one.who.get(),
                self.list.len()
            ));
        }
        sent
    }

    pub(crate) fn len(&self) -> usize {
        self.list.len()
    }
}

fn prefix_of(filter: &Path, road: &str) -> bool {
    let f = filter.as_str();
    if f.is_empty() {
        return true;
    }
    road.strip_prefix(f)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}
