//! 读端：读一格就**前进一步**；架空了就等铃。
//!
//! 与 `hand::Receiver` 的分界：那边一次 `recv` 拿到"别人递来的那一条"；这边是**一条队列的
//! 游标**——`try_recv` 不前进（看一眼），`recv` / `next` 读走才前进。格序号比"我该读的那一号"
//! 大 ⇒ 中间那几格被写者覆盖丢了，跳过并记 `lost`（见 `rack` 头注那张图）。

use core::marker::PhantomData;

use env::{MailFail, PieToken, Wait};
use runtime::core::res::dock::{Dock, View};
use runtime::env::mail::{NolePie, PolePie};

use super::{Ring, SLOT, exact, pop, ring};
use crate::wire::message::Message;

/// **读端**：一条游标 ＋ 一枚铃。
pub struct Reader<M: Message> {
    ring: &'static Ring,
    /// 本域里那一份映射（`Rack::reader` 现取时是 `None`：`Rack` 持着 `Dock`）。
    /// **它只为"持着"而存在**（见 `Writer` 同一格的注）。
    _dock: Option<Dock>,
    /// **我该读的那一号**（绝对序号，从 1 起）。
    next: u64,
    bell: NolePie,
    _m: PhantomData<M>,
}

impl<M: Message> Reader<M> {
    /// 本域里现取一枚读端（`Rack::reader`）：映射归 `Rack`。
    pub fn of(view: View, bell: PieToken) -> Self {
        exact::<M>();
        Self {
            ring: ring(view),
            _dock: None,
            next: 1,
            bell: NolePie::from_token(bell),
            _m: PhantomData,
        }
    }

    /// **对端**那边的读端：拿 `Rack::ship()` 交出的两枚号重建（与 `Receiver::from_token` 同形）。
    /// 页映不进来 ⇒ `None`（此后 `recv` 一律答 `RecvFail::Mail(Denied)`，不猜地址）。
    pub fn from_token(page: PieToken, bell: PieToken) -> Option<Self> {
        exact::<M>();
        let dock = Dock::open(PolePie::from_token(page)).ok()?;
        let view = dock.view();
        Some(Self {
            ring: ring(view),
            _dock: Some(dock),
            next: 1,
            bell: NolePie::from_token(bell),
            _m: PhantomData,
        })
    }

    /// 看一眼：**不前进**。`Ok(None)` = 这一刻没读到。
    pub fn try_recv(&mut self) -> Result<Option<M::In>, RecvFail> {
        let mut frame = [0u8; SLOT];
        match pop(self.ring, &mut self.next, &mut frame) {
            Ok(Some(n)) => {
                let bytes = frame.get(..n).unwrap_or(&[]);
                match M::fetch(bytes) {
                    Some(in_) => Ok(Some(in_)),
                    None => Err(RecvFail::Unread(n)),
                }
            }
            Ok(None) => Ok(None),
            // 这一格被别的读者拿走了（一页一端，不该发生）：如实报，不猜。
            Err(0) => Err(RecvFail::Mail(MailFail::Busy)),
            Err(n) => Err(RecvFail::Unread(n)),
        }
    }

    /// **等铃**。返 `true` = 架上有事（醒来还是要读），`false` = 期限内没等到。
    /// 铃是提示型：一次响可能对应好几条，故**读干为止**是调用方的循环。
    pub fn wait(&self, within: Wait) -> Result<bool, RecvFail> {
        self.bell
            .wait(within)
            .map_err(|e| RecvFail::Mail(e.source))
    }

    /// **等 ＋ 读一条**（与 std 的 `recv` 同位）。
    ///
    /// `within` = 这一次调用等多久（`Wait::POLL` = 只探测一次，**不挂起**）。
    /// 到期仍没有 ⇒ `RecvFail::Empty`。
    pub fn recv(&mut self, within: Wait) -> Result<M::In, RecvFail> {
        let until = crate::communication::deadline(within);
        loop {
            if let Some(one) = self.try_recv()? {
                return Ok(one);
            }
            let remain = crate::communication::remain(until);
            if remain == Wait::POLL {
                return Err(RecvFail::Empty);
            }
            // 等铃：**它只是"有事"**，醒来还得回环里读（可能已被别人读干，或读到的还是
            // 那些丢过的格）。故这里不把铃当成"有一条"的承诺。
            if !self.wait(remain)? {
                return Err(RecvFail::Empty);
            }
        }
    }

    /// 因写者覆盖而丢掉的条数（读者跳过时记）——**与 `Writer::lost` 是同一个数**。
    pub fn lost(&self) -> u64 {
        self.ring.lost.load(core::sync::atomic::Ordering::Relaxed)
    }

    /// 本端那一枚铃的号。
    pub fn bell(&self) -> PieToken {
        self.bell.token()
    }
}

/// 收不回来：三格**分得开**（与 `hand::RecvFail` 同一套词，只多"这一刻没读到"那一格）。
#[derive(Debug)]
pub enum RecvFail {
    /// 期限内没有可取之事（`Wait::POLL` 那一档就是"探测一次"）。
    Empty,
    /// 载体那一层搬不动（`Busy` / `Dead` / `Denied` / `Gone`）。
    Mail(MailFail),
    /// 收到了 `len` 字节，解不动。
    Unread(usize),
}
