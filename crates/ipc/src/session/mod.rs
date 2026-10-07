//! 关系那一支：**装一条路 → 认对端 → 要一枚问话孔**，以及它底下"两枚孔怎么到手"（`establish`）。
//! 两者合成一支，是因为 `Session::open` 就是 `endpoint()` ＋ `hear` ＋ `ask` ——
//! "两枚孔到手"与"一条路装上"是同一件事的两段（见 `establish` 头注那"一手"）。

pub mod establish;
pub use establish::{Endpoint, Held, alive, opened_by};
pub mod exchange;
pub use exchange::{CallFail, Contract};

use env::wire::Field;
use env::{Mark, PieToken, TaskId, Wait};
use ::resource::raw::{Hole};


/// 一条路的名字：**泊位那一格**（`link`）＋ **问话孔那一格**（`ask`）
#[derive(Clone, Copy)]
pub struct Berth {
    /// 这条路（泊位）叫什么
    pub link: Mark,
    /// 这条路上那枚问话孔的记号
    pub ask: Mark,
}

/// 一条装好的会话：本端那一对孔（答话走 `link`、问话走 `talk`）＋ **对端的号**（`host`）
pub struct Session {
    /// 本端**答话**那一枚（本端读）
    pub link: Endpoint,
    /// **问话**那一枚（本端写、对端读）
    pub talk: PieToken,
    /// **对端的号**（"答话的是谁"）
    pub host: TaskId,
}

pub enum Fail {
    /// 这条路没接上：装泊位 / 认对端那一枚 / 收"答话的是谁"——三步任一没成
    Link,
    /// 问话孔铸不出来、或交不出去
    Ask,
}

impl Session {
    /// Send one typed request and receive its typed response within one shared budget.
    pub fn call<C: Contract>(
        &self,
        request: C::Request,
        within: Wait,
    ) -> Result<<C::Response as wire::Message>::In, CallFail> {
        exchange::call::<C>(self, request, within)
    }

    /// 开一条到 `berth` 那条路的会话
    /// `holder` = 客人认的对端 = **它的生我者**：孔交给它，它再转授给那条路上真正的服务
    pub fn open(holder: TaskId, berth: Berth, millis: Wait) -> Result<Session, Fail> {
        let link = establish::endpoint(holder, berth.link, millis).map_err(|_| Fail::Link)?;
        // **认不到对端那一枚 = 这条路没接上**：两侧各装一条、凑齐才算通。
        if link.tx().is_none() {
            return Err(Fail::Link);
        }
        let host = hear(&link, millis).ok_or(Fail::Link)?;
        let talk = ask(host, berth.ask).map_err(|_| Fail::Ask)?;
        Ok(Session { link, talk, host })
    }
}

/// 收下路上那一格：**答话的是谁**（装配侧 `bridge.rs` 的 `tell` 的对偶）
/// 宽度与字节序归 Field 给 TaskId 那一对 `store` / `fetch`
fn hear(link: &Endpoint, millis: Wait) -> Option<TaskId> {
    let mut buf = [0u8; TaskId::WIDTH];
    match Hole::from_raw(link.rx()).pull(&mut buf, millis) {
        Ok((n, _)) if n == TaskId::WIDTH => TaskId::fetch(&buf),
        _ => None,
    }
}

/// 铸**问话孔**并交给对端（本端随即自窄到只写）
/// **一个域一条路只铸一枚**——先找我表里那一枚，有就不铸第二枚。认的是"**本端开的** ＋ 记号"
/// 两格。于是"只铸一枚"从**纪律**变成
/// **构造**：这条路上再也生不出第二枚，而第二枚的症状是"多出来的那枚永远没人读它的推"
fn ask(host: TaskId, mark: Mark) -> Result<PieToken, ()> {
    if let Some(have) = establish::find(me(), mark) {
        return Ok(have);
    }
    // 铸 + 交出读端 + 本端窄到只写：一手就是 establish::give。
    establish::give(host, mark).map_err(|_| ())
}

/// **本端是哪一枚线程**——"这一枚孔是谁开的"那一问要它
/// 不返 `Result`：`SelfId` 那一格恒写 id（生成的入口标了 `#[infallible]`）
fn me() -> TaskId {
    env::unit::self_id()
}
