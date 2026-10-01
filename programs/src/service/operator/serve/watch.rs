//! **订阅册**：谁订了哪条子树、事件往谁那一页里写。
//!
//! # 它为什么住 `serve/` 而不是 `core/`
//! `core` 是"树 ＋ 那几条原语 ＋ 给判据的三条"——它**不认识"订阅"这个词**。订阅是**协议面**
//! 的事（`watch` 那一面收下的一块），故住 `serve`，与 `plate`（提示之路那一支）同侪。
//!
//! # 三件事，各有各的失败处置
//! - `join`：把交来的那两块（页 ＋ 铃）认成**写端**。认不出来 ⇒ `Err(())`，`answer` 答 `Denied`
//!   ——订阅者那一侧拿得到"成没成"，不必猜。
//! - `publish`：**每次都照写**（`rack` 的写端永不挂起：满了按策略丢）。写不动的订阅者
//!   （页没了 / 那一枚封印了）**就地摘掉**：留着只会每一趟都白写一遍。
//! - 撤销：**没有第二条形状**——订阅者退场，它那一页与那一铃随之作废，下一次 `publish` 就摘。

use alloc::vec::Vec;

use env::TaskId;
use protocol::common::path::{Path, PathBuf};
use protocol::communication::rack::{self, Mode, Writer};
use protocol::debug;
use protocol::service::operator::frame::watch::{Event, Kind};
use protocol::service::operator::EntryId;

use crate::service::operator::core::Operator;

/// **一位订阅者**：它是谁 ＋ 它订哪条路 ＋ 事件往哪写。
struct Subscriber {
    /// 订阅者那一枚线程（诊断与"它还在不在"那一问用）
    who: TaskId,
    /// 订的那条路（**空前缀 = 全树**）
    filter: PathBuf,
    /// **写端**：那一页 ＋ 那一枚铃（由 `Wire::Watch` 交来的两个号重建）
    out: Writer<Event>,
}

/// 订阅册。
#[derive(Default)]
pub struct Watchers {
    list: Vec<Subscriber>,
}

impl Watchers {
    pub const fn new() -> Watchers {
        Watchers { list: Vec::new() }
    }

    /// **收一位订阅者**：把交来的页与铃认成写端，记下它订的那条路。
    ///
    /// 页映不进来 / 铃不在表里 ⇒ `Err(())`（调用方答 `Denied`）。**已订过的同一位不再记第二条**：
    /// 一条会话只该订一次（重复订会让同一次改动写两份）。
    pub fn join(
        &mut self,
        who: TaskId,
        road: &Path,
        page: env::PieToken,
        bell: env::PieToken,
    ) -> Result<(), ()> {
        let Some(out) = Writer::from_token(page, bell, Mode::Oldest) else {
            debug!("operator: watch page not mappable who={}", who.get());
            return Err(());
        };
        if let Some(at) = self.list.iter().position(|one| one.who == who) {
            self.list.swap_remove(at);
        }
        let filter = road.to_path_buf();
        self.list.try_reserve(1).map_err(|_| ())?;
        // **收下了就报一行（release 也看得见）**：这一行是"树上真记下了这一位"的唯一直接证据
        // ——订阅者手里那一句 `OK` 只能证明"对面答了"，证明不了"它记在哪一格上"。
        let at = self.list.len();
        self.list.push(Subscriber { who, filter, out });
        protocol::debug::put(&alloc::format!(
            "operator: watch joined who={} road={} watchers={}",
            who.get(),
            self.list[at].filter,
            self.len()
        ));
        Ok(())
    }

    /// **把一次改动发给订得起的人**：只对"`filter` 是这条路的（前缀）"的那些写。
    ///
    /// 写不动的就地摘掉（**不留死订阅者**：留着只会每一趟都白写一遍）。返写出去几份。
    pub fn publish(&mut self, ev: &Event) -> usize {
        let road = ev.road.as_str();
        let mut sent = 0usize;
        let mut dead: Vec<usize> = Vec::new();
        for (at, one) in self.list.iter_mut().enumerate() {
            if !prefix_of(&one.filter, road) {
                continue;
            }
            match one.out.send(ev.clone()) {
                Ok(()) => sent += 1,
                // `Mode::Oldest` 下满/丢都不是"写不动"——只有载体那一格的失败才算这位没了。
                Err(rack::SendFail::Full) | Err(rack::SendFail::TooLong) => sent += 1,
                Err(fail) => {
                    debug!(
                        "operator: watch dropped who={} why={:?}",
                        one.who.get(),
                        fail
                    );
                    dead.push(at);
                }
            }
        }
        // 从后往前摘（下标不动）。
        for at in dead.into_iter().rev() {
            self.list.swap_remove(at);
        }
        // **照实量**（临时读数）：这一次改动写给几位、册里现在几位——用来判"死订户还在不在册里、
        // 还在不在被写"。
        protocol::debug::put(&alloc::format!(
            "operator: watch publish watchers={} sent={sent}",
            self.list.len()
        ));
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
pub fn event_at(
    tree: &Operator,
    kind: Kind,
    id: EntryId,
    owner: TaskId,
    road: Option<PathBuf>,
) -> Option<Event> {
    let road = match road {
        Some(road) => road,
        None => tree.road_to(id)?,
    };
    Some(Event {
        kind,
        road,
        id,
        owner,
    })
}
