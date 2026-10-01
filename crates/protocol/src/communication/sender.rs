//! 这一路流的那一种报由类型参数说。
//! ```text

use core::marker::PhantomData;

use env::{HoleDir, MailFail, MailResult, PieToken, Wait};

use crate::wire::message::Message;
use runtime::env::mail::HolePie;

/// **我推的那一枚孔** ＋ 这一路流的那一种报（类型）＋ 那一格缓冲 ＋ 还挂着的那只手
pub struct Sender<M: Message> {
    /// 写端那一枚（`None` = 还没有）
    hole: Option<PieToken>,
    /// 编报那一格：地址在整个借用期里不动（见文件头③）
    buf: M::Buf,
    /// **还挂在孔上的那只手**（`None` = 空着）
    hand: Option<PieToken>,
    _m: PhantomData<M>,
}

impl<M: Message> Sender<M> {
    /// 空格：**没有写端**（`send` 会答 SendFail::Unbound）。`const` 是给"放进结构体里当一格"
    /// 那些用到上的（`Guest` 那一格）
    pub const fn new() -> Self {
        Self {
            hole: None,
            buf: M::EMPTY,
            hand: None,
            _m: PhantomData,
        }
    }

    /// 认下一枚**别人给的**号（服务端那一侧：孔是对方铸的、交给我的）
    pub fn from_token(hole: PieToken) -> Self {
        Self {
            hole: Some(hole),
            buf: M::EMPTY,
            hand: None,
            _m: PhantomData,
        }
    }

    pub fn send(&mut self, msg: M) -> Result<(), SendFail> {
        let Some(hole) = self.hole else {
            return Err(SendFail::Unbound);
        };
        self.reclaim().map_err(|e| SendFail::Mail(e.source))?;
        let Some(n) = msg.store(self.buf.as_mut()) else {
            return Err(SendFail::TooLong);
        };
        let bytes = self.buf.as_ref().get(..n).ok_or(SendFail::TooLong)?;
        HolePie::from_token(hole)
            .push(bytes, Wait::POLL)
            .map_err(|e| SendFail::Mail(e.source))?;
        self.hand = Some(hole);
        Ok(())
    }

    /// **等这只手下线**：送到（孔回到空）或孔封印（`Err(Dead)`）或孔不见了（`Gone`）
    /// **没有期限**——见文件头②。已经空着 ⇒ 当场 `Ok`（零代价）
    pub fn reclaim(&mut self) -> MailResult<()> {
        let Some(hole) = self.hand.take() else {
            return Ok(());
        };
        let r = HolePie::from_token(hole).wait(HoleDir::Push, Wait::Forever);
        if let Err(e) = &r {
            crate::debug!(
                "mail: reclaim miss hole={} code={}",
                hole.get(),
                e.source.code()
            );
        }
        r.map(|_| ())
    }

    /// **非阻塞收口**：上一只手**已经被取走** ⇒ 放下那一格、答 `true`；还压着 ⇒ 答 `false`
    /// **它与 reclaim 只差一个字：期限。** `reclaim` 等 `Forever`——写端**必须**替这段缓冲收尾
    pub fn settle(&mut self) -> bool {
        let Some(hole) = self.hand else {
            return true;
        };
        if let Ok(true) = HolePie::from_token(hole).wait(HoleDir::Push, Wait::AtMost(0)) {
            self.hand = None;
            return true;
        }
        false
    }

    /// 还挂着吗（诊断：不碰内核就能问）
    pub const fn outstanding(&self) -> bool {
        self.hand.is_some()
    }

    pub fn hole(&self) -> Option<PieToken> {
        self.hole
    }
}

impl<M: Message> Default for Sender<M> {
    fn default() -> Self {
        Self::new()
    }
}

/// **落出作用域 = 收口**：还挂着那只手就等它下线
/// 这是"递出即走"能安全成立的那一半（见文件头④）：`send` 不睡，代价是"这段字节还欠着"，而
impl<M: Message> Drop for Sender<M> {
    fn drop(&mut self) {
        if self.hand.is_some() {
            let _ = self.reclaim();
        }
    }
}

/// 递不出去：两层**分得开**
/// - SendFail::TooLong = **编不进本族的缓冲**（M::Buf 就是本族最长那一枚，故这一支只在
/// 类型被写错时才到得了——不 `panic`、如实报）
/// - SendFail::Unbound = **没有写端**（对端那一枚还没认到）
/// - SendFail::Mail = **搬不动**，原样的域词汇（`Busy` / `Dead` / `Denied` / `Gone`）
/// **不另造一套码**：Mail 域的词表是它的失败域，这一层只把"哪一步失败"说清，不换词
pub enum SendFail {
    Unbound,
    TooLong,
    Mail(MailFail),
}
