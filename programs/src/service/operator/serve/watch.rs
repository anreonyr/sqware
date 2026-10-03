//! **订阅册**：谁订了哪条子树、事件往谁那一枚孔上递。
//!
//! # 它为什么住 `serve/` 而不是 `core/`
//! `core` 是"树 ＋ 那几条原语 ＋ 给判据的三条"——它**不认识"订阅"这个词**。订阅是**协议面**
//! 的事（`watch` 那一面收下的一块），故住 `serve`，与 `plate`（提示之路那一支）同侪。
//!
//! # 三件事，各有各的失败处置
//! - `join`：把交来的那枚孔记下。认不下（同一位重复订 / 备不下）⇒ `Err(())`，`answer` 答
//!   `Denied`——订阅者那一侧拿得到"成没成"，不必猜。
//! - `publish`：**一次改动只序列化一次**（写进持树者自己那一具架的下一格），再对每位订得起的
//!   人**递一只手**（只登记"那一格在哪"，不复制字节）。递不进去的三档分得开：队列满 ⇒ 这次对
//!   他丢；**`Dead`／`Gone` ⇒ 这一位没了，就地摘掉**（写之前就知道，故他挂不住树）。
//! - 撤销：**没有第二条形状**——订阅者退场，它那一枚孔随之作废，下一次 `publish` 就摘。
//!
//! # 字节为什么住持树者这儿（与从前那一形的分界）
//! 从前订阅者交来**一页**（`Pole`），事件由持树者直接写进那一页：树上因此按订户数挂着 N 段
//! 映射，而订户退场之后那一段**谁也不回收**（实测：`watchers` 从 4 没降过、写一直成功——
//! 是"只增不减"，不是"写已撤的图"）。改成一枚孔之后，字节住**持树者自己的**那一具架
//! （一页，与订户数无关），跨域只走"手"——手是发送方那段内存的登记（**不含字节**），
//! 取走那一刻内核复制一次。

use alloc::vec::Vec;

use env::{PieToken, TaskId, Wait};
use protocol::common::path::{Path, PathBuf};
use protocol::communication::rack::{self, Mode, Rack};
use protocol::service::operator::EntryId;
use protocol::service::operator::frame::watch::{Event, Kind};
use runtime::env::mail::HolePie;

use crate::service::operator::core::Operator;

/// **一位订阅者**：它是谁 ＋ 它订哪条路 ＋ 事件往哪一枚孔上递。
struct Subscriber {
    /// 订阅者那一枚线程（诊断与"它还在不在"那一问用）
    who: TaskId,
    /// 订的那条路（**空前缀 = 全树**）
    filter: PathBuf,
    /// **递手的那一枚孔**（由 `Wire::Watch` 交来的号认下）
    hole: PieToken,
}

/// 订阅册 ＋ **持树者自己那一具架**（事件的字节住它那一页里）。
pub struct Watchers {
    list: Vec<Subscriber>,
    /// 事件那一具架：一次改动序列化进下一格，手只指向**那一格**（零复制）。
    rack: Rack<Event>,
    /// 写端（本地那一手：只往自己这一页里落一格，不碰别的域）。
    writer: rack::Writer<Event>,
    /// **事件的号**（从 1 起：与环里的格一一对应，也是订户认出"丢了几条"的凭据）。
    seq: u64,
}

impl Watchers {
    /// 起手：开**一具自己的架**（一页 ＋ 头 ＋ `CAP` 格 ＋ 一枚没人等的铃）。
    ///
    /// 铃在这一形里没有读者（唤醒由孔那一侧承担：递手就唤），每次改动白响一下——**1 次/改动**。
    pub fn open() -> Result<Watchers, ()> {
        let rack = Rack::<Event>::open(Mode::Oldest).map_err(|_| ())?;
        let writer = rack.writer();
        // 读数（release 也看得见）：**树上只有这一页**，与订户数无关。
        protocol::debug::put(&alloc::format!(
            "operator: event rack page={}B cap={} mode=Oldest",
            rack::SIZE,
            rack::CAP
        ));
        Ok(Watchers {
            list: Vec::new(),
            rack,
            writer,
            seq: 0,
        })
    }

    /// **收一位订阅者**：记下它那一枚孔与它订的那条路。
    ///
    /// **判据是"同一位的同一条路只留一条"**（不是"一位只留一条"）：同一位订两条不同的路是正当的
    /// （各有一条自己的孔、各收各的）；而**同一位同一条路订两次**会让同一次改动递两份，故拒。
    /// 孔那一侧**不预检**：孔不在表里 / 封印了，`publish` 那一手会当场答 `Dead`／`Gone` 就地摘掉
    /// ——省一趟往返，也省一格"认孔"的状态。
    pub fn join(&mut self, who: TaskId, road: &Path, hole: PieToken) -> Result<(), ()> {
        if self
            .list
            .iter()
            .any(|one| one.who == who && one.filter.as_str() == road.as_str())
        {
            return Err(());
        }
        self.list.try_reserve(1).map_err(|_| ())?;
        let filter = road.to_path_buf();
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

    /// **把一次改动递给订得起的人**：① 写进自己那一具架（本地序列化一次）→ ② 取那一格的字节
    /// → ③ 逐位**递一只手**（`Push` 只登记指针、不复制）。
    ///
    /// 返递出去几份。**写者永不挂起**：队列满就丢这一次（通知不是账，订户醒来照样 `list`）。
    pub fn publish(&mut self, mut ev: Event) -> usize {
        // ① 号由本册给（它是号的所有者），再落进环的下一格。
        self.seq += 1;
        ev.seq = self.seq;
        if self.writer.send(ev.clone()).is_err() {
            // 装不进一格（本族的报比 `SLOT` 还长）——类型那一关就过不去，报一行便于对账。
            protocol::debug::put(&alloc::format!(
                "operator: watch event too long seq={}",
                ev.seq
            ));
            return 0;
        }
        // ② 那一格的载荷：手就指向它（**不复制**）。
        let slot = self.rack.slot(ev.seq);
        // ③ 逐位递手。
        let road = ev.road.as_str();
        let mut sent = 0usize;
        let mut dead: Vec<usize> = Vec::new();
        for (at, one) in self.list.iter_mut().enumerate() {
            if !prefix_of(&one.filter, road) {
                continue;
            }
            match HolePie::from_token(one.hole).push(slot, Wait::POLL) {
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
        seq: 0,
        kind,
        road,
        id,
        owner,
    })
}
