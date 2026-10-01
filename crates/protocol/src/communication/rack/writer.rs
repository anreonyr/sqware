//! 写端：**永不挂起**。满了按 `Mode` 丢一头。
//!
//! 与 `hand::Sender` 的分界：那边 `send` 之后还要 `reclaim`（等这只手下线，无期等）；
//! 这边 `send` 落地即走——丢了的算在 `dropped` / `lost` 两个数上，不由发送方等。

use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use env::{PieToken, Wait};
use runtime::core::res::dock::{Dock, View};
use runtime::env::mail::{NolePie, PolePie};

use super::{Mode, Ring, SLOT, exact, push, ring};
use crate::wire::message::Message;

/// **写端**：一枚页上的环 ＋ 一枚铃 ＋ 本族那只编报缓冲。
///
/// 名字与 `std::sync::mpsc::SyncSender` 同位——但**不阻塞**：容量满了不是"等"，而是按
/// [`Mode`] 丢（见 `SendFail::Full`）。
pub struct Writer<M: Message> {
    ring: &'static Ring,
    /// 本域里那一份映射（`Rack::writer` 现取时是 `None`：`Rack` 持着 `Dock`）。
    dock: Option<Dock>,
    /// 编报那一格：地址在整个持有期里不动（`store` 写它、`push` 读它）。
    buf: M::Buf,
    mode: Mode,
    bell: NolePie,
    _m: PhantomData<M>,
}

impl<M: Message> Writer<M> {
    /// 本域里现取一枚写端（`Rack::writer`）：映射归 `Rack`，本端只借视图那一对数。
    pub fn of(view: View, mode: Mode, bell: PieToken) -> Self {
        exact::<M>();
        Self {
            ring: ring(view),
            dock: None,
            buf: M::EMPTY,
            mode,
            bell: NolePie::from_token(bell),
            _m: PhantomData,
        }
    }

    /// **对端**那边的写端：拿 `Rack::ship()` 交出的两枚号重建（与 `Sender::from_token` 同形）。
    ///
    /// 本端自己开一份映射并**持着它**（与驱动那一侧 `Dock::open` 同一条手：谁 open 谁持有）。
    /// 页映不进来 ⇒ `None`——此后 `send` 一律答 `SendFail::Mail(Denied)`，不猜地址。
    pub fn from_token(page: PieToken, bell: PieToken, mode: Mode) -> Option<Self> {
        exact::<M>();
        let dock = Dock::open(PolePie::from_token(page)).ok()?;
        let view = dock.view();
        Some(Self {
            ring: ring(view),
            dock: Some(dock),
            buf: M::EMPTY,
            mode,
            bell: NolePie::from_token(bell),
            _m: PhantomData,
        })
    }

    /// **落一格**：放进去了答 `Ok`；按策略丢了答 `Err(SendFail::Full)`（**不是失败**，
    /// 是"这一格没进架"——丢掉的数在 `lost` / `dropped` 上）。
    pub fn send(&mut self, msg: M) -> Result<(), SendFail> {
        let Some(n) = msg.store(self.buf.as_mut()) else {
            return Err(SendFail::TooLong);
        };
        if n == 0 || n > SLOT {
            return Err(SendFail::TooLong);
        }
        let bytes = self.buf.as_ref().get(..n).ok_or(SendFail::TooLong)?;
        // 正文在 `rack::push` 那一处（与模块内的用例同一份代码）。
        push(self.ring, self.mode, bytes).map_err(|()| SendFail::Full)?;
        // 铃是"有事"（提示型）：已响即 `Busy`，不是错——读者醒来就会把架读干。
        let _ = self.bell.ring();
        Ok(())
    }

    /// 非阻塞的别名（与 std 同名）：本实现里 `send` 本来就只尝试一次。
    pub fn try_send(&mut self, msg: M) -> Result<(), SendFail> {
        self.send(msg)
    }

    /// 等铃（**写者不该用**：它只响、不等。留给"同一域里要等答复"的诊断口）。
    pub fn wait(&self, within: Wait) -> bool {
        self.bell.wait(within).unwrap_or(false)
    }

    /// 按策略丢掉的条数（`Mode::Newest` 会加）。
    pub fn dropped(&self) -> u64 {
        self.ring.dropped.load(Ordering::Relaxed)
    }

    /// 因写者覆盖而丢掉的条数（读者跳过时记）。
    pub fn lost(&self) -> u64 {
        self.ring.lost.load(Ordering::Relaxed)
    }

    /// 现在架上有几条（**诊断用**：与读者那一刻看到的未必同值）。
    pub fn pending(&self) -> u64 {
        let w = self.ring.write.load(Ordering::Acquire);
        let r = self.ring.read.load(Ordering::Acquire);
        w.wrapping_sub(r)
    }

    /// 本端那一枚铃的号（要交给别人听时用）。
    pub fn bell(&self) -> PieToken {
        self.bell.token()
    }
}

/// 递不出去：三格**分得开**（与 `hand::SendFail` 同一套词，只多"这一格没进架"那一格）。
pub enum SendFail {
    /// **这一格没进架**：容量满了，按 [`Mode`] 丢了（丢掉的数在 `lost` / `dropped` 上）。
    Full,
    /// 编不进本族的缓冲（`M::Buf` 就是本族最长那一枚，故这一支只在类型被写错时才到得了）。
    TooLong,
    /// **没有写端**（对端那一枚还没认到）——沿用 `hand` 那一档的词。
    Unbound,
    /// 搬不动，原样的域词汇（`Busy` / `Dead` / `Denied` / `Gone`）。
    Mail(env::MailFail),
}
