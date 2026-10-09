//! 写端：**永不挂起**。满了按 `Mode` 丢一头。
//!
//! 与 `hand::Sender` 的分界：那边 `send` 之后还要 `reclaim`（等这只手下线，无期等）；
//! 这边 `send` 落地即走——丢了的算在 `lost` / `dropped` 两个数上，不由发送方等。
//!
//! **两个数各说各的**（见 [`super::ring`] 的"三个数"那一节）：`lost` = `Mode::Oldest` 下顶掉
//! 未读格的枚数，`dropped` = `Mode::Newest` 下没进架的枚数。两者都是**写端**记的。

use core::marker::PhantomData;

use env::{PieToken, Wait};
use ::resource::dock::{Dock, View};

use super::Mode;
use super::bell::Bell;
use super::ring::{Ring, SLOT, depth, dropped, exact, lost, push, ring};
use wire::Message;

/// **写端**：一枚页上的环 ＋ 一枚铃 ＋ 本族那只编报缓冲。
///
/// 名字与 `std::sync::mpsc::SyncSender` 同位——但**不阻塞**：容量满了不是"等"，而是按
/// [`Mode`](super::Mode) 丢（见 [`SendFail::Full`]）。
pub struct Writer<M: Message> {
    ring: &'static Ring,
    /// 本域里那一份映射（`Rack::writer` 现取时是 `None`：`Rack` 持着 `Dock`）。
    /// **它只为"持着"而存在**：`reader`/`writer` 用的是 `view()` 那一对数，而映射要靠这一格
    /// 活着（放了就是撤图）——故下划线起头，说明"这一格没有读点"。
    _dock: Option<Dock>,
    /// 编报那一格：地址在整个持有期里不动（`store` 写它、`push` 读它）。
    buf: M::Buf,
    mode: Mode,
    bell: Bell,
    _m: PhantomData<M>,
}

impl<M: Message> Writer<M> {
    /// 本域里现取一枚写端（`Rack::writer`）：映射归 `Rack`，本端只借视图那一对数。
    /// `page` = **那一枚页**（页上那一位就是铃：落完一格响一下）。
    pub fn of(view: View, mode: Mode, page: PieToken) -> Self {
        exact::<M>();
        Self {
            ring: ring(view),
            _dock: None,
            buf: M::EMPTY,
            mode,
            bell: Bell::from_raw(page),
            _m: PhantomData,
        }
    }

    /// **对端**那边的写端：拿 [`super::Rack::ship`] 交出的**那一枚号**重建（与 `Sender::from_raw` 同形）。
    ///
    /// 本端自己开一份映射并**持着它**（与驱动那一侧 `Dock::open` 同一条手：谁 open 谁持有）。
    /// 页映不进来 ⇒ `None`——此后 `send` 一律答 `SendFail::Mail(Denied)`，不猜地址。
    pub fn from_raw(page: PieToken, mode: Mode) -> Option<Self> {
        exact::<M>();
        let dock = match Dock::open(page) {
            Ok(dock) => dock,
            Err(fail) => {
                // **release 也看得见**：写端映不进来是"订阅成了但发不出去"那一格的头号成因。
                crate::debug::put(&alloc::format!("rack: writer no view {:?}", fail));
                return None;
            }
        };
        let view = dock.view();
        Some(Self {
            ring: ring(view),
            _dock: Some(dock),
            buf: M::EMPTY,
            mode,
            bell: Bell::from_raw(page),
            _m: PhantomData,
        })
    }

    /// Publish only when a free slot exists; a full rack retains all queued frames.
    pub fn send_when_ready(&mut self, msg: &M) -> Result<bool, SendFail> {
        if super::ring::depth(self.ring) >= super::ring::CAP as u64 {
            let _ = self.bell.hush_space();
            if super::ring::depth(self.ring) >= super::ring::CAP as u64 { return Ok(false); }
        }
        self.send(msg)?;
        Ok(true)
    }

    /// Clear the producer's progress hint when no retained frame needs it.
    pub fn hush_space(&self) { let _ = self.bell.hush_space(); }

    /// Publish once, applying the configured full-rack policy.
    pub fn send(&mut self, msg: &M) -> Result<(), SendFail> {
        let Some(n) = msg.store(self.buf.as_mut()) else {
            return Err(SendFail::TooLong);
        };
        if n == 0 || n > SLOT {
            return Err(SendFail::TooLong);
        }
        let bytes = self.buf.as_ref().get(..n).ok_or(SendFail::TooLong)?;
        // 正文在 `rack::ring::push` 那一处（与模块内的用例同一份代码）。
        push(self.ring, self.mode, bytes).map_err(|()| SendFail::Full)?;
        // 铃是"有事"（提示型）：已响即 `Busy`，不是错——读者醒来就会把架读干。
        let _ = self.bell.ring();
        Ok(())
    }

    /// 非阻塞的别名（与 std 同名）：本实现里 `send` 本来就只尝试一次。
    pub fn try_send(&mut self, msg: &M) -> Result<(), SendFail> {
        self.send(msg)
    }

    /// 等铃（**写者不该用**：它只响、不等。留给"同一域里要等答复"的诊断口）。
    pub fn wait(&self, within: Wait) -> bool {
        self.bell.wait(within).unwrap_or(false)
    }

    /// 按 `Mode::Newest` 丢掉的条数（没进架的那些）。
    pub fn dropped(&self) -> u64 {
        dropped(self.ring)
    }

    /// 按 `Mode::Oldest` 顶掉的**未读**格数（写端记）。
    pub fn lost(&self) -> u64 {
        lost(self.ring)
    }

    /// 现在架上有几条（**诊断用**：与读者那一刻看到的未必同值）。
    pub fn pending(&self) -> u64 {
        depth(self.ring)
    }

    /// 本端那一枚铃的号（要交给别人听时用）。
    pub fn bell(&self) -> PieToken {
        self.bell.token()
    }
}

/// 递不出去：三格**分得开**（与 `hand::SendFail` 同一套词，只多"这一格没进架"那一格）。
#[derive(Debug)]
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
