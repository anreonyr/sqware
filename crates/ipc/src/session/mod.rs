//! A session uses a fresh transport handshake and a fresh request capability per open.

pub mod establish;
pub use establish::{DiscoveryFail, Endpoint, Held, alive, opened_by};
pub mod exchange;
pub use exchange::CallFail;
pub use wire::Contract;
mod state;

extern crate alloc;
use ::resource::raw::{self, Hole};
use alloc::sync::Arc;
use env::pie;
use env::wire::Field;
use env::{Mark, PieToken, TaskId, Wait};

/// 一条路的名字：**泊位那一格**（`link`）＋ **问话孔那一格**（`ask`）
#[derive(Clone, Copy)]
pub struct Berth {
    /// 这条路（泊位）叫什么
    pub link: Mark,
    /// 这条路上那枚问话孔的记号
    pub ask: Mark,
}

/// 一条装好的会话：本端那一对孔（答话走 `link`、问话走 `talk`）＋ **对端的号**（`host`）
#[derive(Clone)]
pub struct Session {
    /// 本端**答话**那一枚（本端读）
    link: Endpoint,
    /// **问话**那一枚（本端写、对端读）
    talk: PieToken,
    /// **对端的号**（"答话的是谁"）
    host: TaskId,
    state: Arc<state::State>,
    _owned: Option<Arc<Owned>>,
}

struct Owned {
    rx: PieToken,
    talk: PieToken,
}

impl Drop for Owned {
    fn drop(&mut self) {
        if self.talk != PieToken::NONE {
            let _ = pie::release(self.talk);
        }
        let _ = pie::release(self.rx);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// 这条路没接上：装泊位 / 认对端那一枚 / 收"答话的是谁"——三步任一没成
    Link,
    /// 问话孔铸不出来、或交不出去
    Ask,
}

impl Session {
    /// The peer that owns the response side of this session.
    pub const fn host(&self) -> TaskId {
        self.host
    }

    /// The request hole used by this session.
    pub const fn talk(&self) -> PieToken {
        self.talk
    }

    /// The local response endpoint shared by aliases of this session.
    pub const fn link(&self) -> Endpoint {
        self.link
    }

    /// Import raw endpoint capabilities after verifying ownership of the local reply hole.
    pub fn from_raw(link: Endpoint, talk: PieToken, host: TaskId) -> Result<Self, Fail> {
        if !state::valid_reply(link.rx(), host) {
            return Err(Fail::Link);
        }
        Ok(Self {
            link,
            talk,
            host,
            state: state::new_state(),
            _owned: None,
        })
    }

    /// Send one typed request and receive its typed response within one shared budget.
    pub fn call<C: Contract>(
        &self,
        request: C::Request,
        within: Wait,
    ) -> Result<<C::Response as wire::Message>::In, CallFail> {
        exchange::call::<C>(self, request, within)
    }

    /// 开一条到 `berth` 那条路的会话
    /// Each call creates a new local reply hole and a new request hole.
    pub fn open(holder: TaskId, berth: Berth, millis: Wait) -> Result<Session, Fail> {
        let until = crate::time::deadline(millis);
        let link = establish::lend(holder, berth.link).map_err(|_| Fail::Link)?;
        let mut owned = Owned {
            rx: link.rx(),
            talk: PieToken::NONE,
        };

        // Bootstrap is exactly (selected host, local transport token), both fixed-width fields.
        let mut bootstrap = [0u8; TaskId::WIDTH + PieToken::WIDTH];
        let (len, sender) = Hole::from_raw(link.rx())
            .pull(&mut bootstrap, crate::time::remain(until))
            .map_err(|_| Fail::Link)?;
        if len != bootstrap.len() || sender != holder {
            return Err(Fail::Link);
        }
        let host = TaskId::fetch(&bootstrap[..TaskId::WIDTH]).ok_or(Fail::Link)?;
        let transport = PieToken::fetch(&bootstrap[TaskId::WIDTH..]).ok_or(Fail::Link)?;
        if host == TaskId::new(0) || !raw::alive(transport) {
            return Err(Fail::Link);
        }
        if !matches!(raw::reserve(transport), Ok((vestor, owner, mark))
            if vestor == holder && owner == holder && mark == berth.link)
        {
            return Err(Fail::Link);
        }

        let (talk, ask_seed) = establish::give_at(host, berth.ask).map_err(|_| Fail::Ask)?;
        owned.talk = talk;
        let mut ask_bytes = [0u8; PieToken::WIDTH];
        ask_seed.store(&mut ask_bytes);
        Hole::from_raw(transport)
            .push(&ask_bytes, crate::time::remain(until))
            .map_err(|_| Fail::Link)?;

        let mut ack = [0u8; 1];
        let (len, sender) = Hole::from_raw(link.rx())
            .pull(&mut ack, crate::time::remain(until))
            .map_err(|_| Fail::Link)?;
        if len != 1 || ack[0] != wire::OK || sender != host {
            return Err(Fail::Link);
        }

        Ok(Session {
            link,
            talk,
            host,
            state: state::new_state(),
            _owned: Some(Arc::new(owned)),
        })
    }
}
