//! 订阅者、路径过滤与事件投递。
//! 事件编码一次；每次 Push 将字节复制进内核，编码缓冲可立即复用。
//! 满队列丢弃本次通知，失效孔移除订阅者。

use runtime::schedule::{Progress, Res, ResMut};

use alloc::vec::Vec;

use env::{PieToken, TaskId, Wait};
use protocol::common::path::{Path, PathBuf};
use protocol::system::operator::Event;
use protocol::wire::message::Message;

use crate::system::operator::core::Operator;
use runtime::core::res::pie::{HolePie};

/// **一位订阅者**：它是谁 ＋ 它订哪条路 ＋ 事件往哪一枚孔上递。
struct Subscriber {
    /// 订阅者的 task。
    who: TaskId,
    /// 订的那条路（**空前缀 = 全树**）
    filter: PathBuf,
    /// **递手的那一枚孔**（由 `Wire::Watch` 交来的号认下）
    hole: PieToken,
}

/// 订阅者列表、复用编码缓冲与事件序号。
pub struct Watchers {
    list: Vec<Subscriber>,
    buffer: <Event as Message>::Buf,
    seq: u64,
}

impl Watchers {
    pub fn new() -> Self {
        Self {
            list: Vec::new(),
            buffer: Event::EMPTY,
            seq: 0,
        }
    }

    /// **收一位订阅者**：记下它那一枚孔与它订的那条路。
    ///
    /// **判据是"同一位的同一条路只留一条"**（不是"一位只留一条"）：同一位订两条不同的路是正当的
    /// （各有一条自己的孔、各收各的）；而**同一位同一条路订两次**会让同一次改动递两份，故拒。
    /// 孔那一侧**不预检**：孔不在表里 / 封印了，`publish` 那一手会当场答 `Dead`／`Gone` 就地摘掉
    /// ——省一趟往返，也省一格"认孔"的状态。
    pub fn join(&mut self, subscription: Subscription) -> Result<(), ()> {
        let Subscription { who, road, hole } = subscription;
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
        // **收下了就报一行（release 也看得见）**：这一行是"树上真记下了这一位"的唯一直接证据
        // ——订阅者手里那一句 `OK` 只能证明"对面答了"，证明不了"它记在哪一格上"。
        let at = self.list.len() - 1;
        protocol::debug::put(&alloc::format!(
            "operator: watch joined who={} road={} watchers={}",
            who.get(),
            self.list[at].filter,
            self.len()
        ));
        Ok(())
    }

    /// 编码一次并投递给匹配的订阅者；队列满时丢弃本次通知。
    pub fn publish(&mut self, mut ev: Event) -> usize {
        self.seq += 1;
        ev.seq = self.seq;
        let Some(size) = ev.store(&mut self.buffer) else {
            protocol::debug::put(&alloc::format!(
                "operator: watch event too long seq={}",
                ev.seq
            ));
            return 0;
        };
        // Push 返回后字节已归内核，不依赖下一次发布的缓冲内容。
        let bytes = &self.buffer[..size];
        let road = ev.road.as_str();
        let mut sent = 0usize;
        let mut dead: Vec<usize> = Vec::new();
        for (at, one) in self.list.iter_mut().enumerate() {
            if !prefix_of(&one.filter, road) {
                continue;
            }
            match HolePie::from_token(one.hole).push(bytes, Wait::POLL) {
                Ok(()) => sent += 1,
                // 队列满：这一次对这位丢。**不摘他**——他还在，只是没跟上（醒来 `list` 即可）。
                Err(e) if e.source.is_busy() => {}
                // 孔没了 / 推不动（`Denied`：交出去的那一枚随他退场被摘）：这一位没了 ⇒ 记下。
                Err(_) => dead.push(at),
            }
        }
        // 从后往前摘（下标不动）：**摘完之后再报**——读数里的 `watchers` 才是摘完的册。
        for at in dead.into_iter().rev() {
            let one = self.list.swap_remove(at);
            protocol::debug::put(&alloc::format!(
                "operator: watch dropped who={} watchers={}",
                one.who.get(),
                self.list.len()
            ));
        }
        sent
    }

    /// 现在有几位订阅者（`join` 那一行读数用它）。
    pub fn len(&self) -> usize {
        self.list.len()
    }
}

/// **`filter` 是 `road` 的前缀吗**（空前缀 = 全树；按**整段**比，`svc/a` 不匹配 `svc/ab`）。
fn prefix_of(filter: &Path, road: &str) -> bool {
    let f = filter.as_str();
    if f.is_empty() {
        return true;
    }
    road.strip_prefix(f)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// **把一次改动拼成一条事件**——路那一段：**剪掉那一档用剪之前记下的**（`change.road`），
/// 其余三档从**号**现走（`Operator::road_to`）。
///
/// 这是"事件里带整条路"那一条纪律的落点：订阅者按它做前缀过滤，故路必须是**从根写起**的。
/// 走不出路（号不在树上）⇒ `None`：**一条路是假的事件比没有更坏**。
///
/// **号那一格留 `0`**：它由订阅册那一手盖（[`Watchers::publish`] 才是号的持有者——号与环里的
/// 格一一对应，故只能由落格那一手给）。
pub struct Subscription {
    pub who: TaskId,
    pub road: PathBuf,
    pub hole: PieToken,
}
pub fn event_at(tree: &Operator, change: crate::system::operator::core::Change) -> Option<Event> {
    let road = match change.road {
        Some(road) => road,
        None => tree.road_to(change.id)?,
    };
    Some(Event {
        seq: 0,
        kind: change.kind,
        road,
        id: change.id,
        owner: change.owner,
    })
}

use super::{Fail, answer::Output, session::Request};
use protocol::{
    system::operator as ocall,
};
pub(super) fn subscribe(
    mut request: ResMut<Request>,
    mut watchers: ResMut<Watchers>,
    mut out: ResMut<Output<ocall::Union>>,
) -> Result<Progress, super::Fail> {
    if let Some(incoming) = &mut request.0 {
        if matches!(incoming.ask, Some(ocall::Wire::Watch { .. })) {
            let Some(ocall::Wire::Watch { road, hole }) = incoming.ask.take() else {
                unreachable!();
            };
            out.reply = Some(
                match watchers.join(Subscription {
                    who: incoming.guest.who(),
                    road,
                    hole,
                }) {
                    Ok(()) => ocall::Union::Status(ocall::OK),
                    Err(()) => ocall::Union::Status(ocall::DENIED),
                },
            );
        }
    }
    Ok(Progress::Done)
}
pub(super) fn emit<T: 'static>(
    tree: Res<Operator>,
    mut watchers: ResMut<Watchers>,
    mut out: ResMut<Output<T>>,
) -> Result<Progress, Fail> {
    for change in out.changes.drain(..) {
        if let Some(event) = event_at(&tree, change) {
            let _ = watchers.publish(event);
        }
    }
    Ok(Progress::Done)
}
