//! 这一路流的那一种报由类型参数说。
//! ```text

use core::marker::PhantomData;

use env::{HoleDir, MailFail, MailResult, PieToken, Wait};

use crate::wire::message::Message;
use runtime::env::mail::HolePie;

/// **我推的那一枚孔** ＋ 这一路流的那一种报（类型）＋ 那一格缓冲 ＋ **我还排着几只**。
pub struct Sender<M: Message> {
    /// 写端那一枚（`None` = 还没有）
    hole: Option<PieToken>,
    /// 编报那一格：地址在整个借用期里不动（见文件头③）。**只服务 [`Sender::send`] 那一档**。
    buf: M::Buf,
    /// **我还排着几只**（孔那头说的数：`Peek` 的第三格）。
    ///
    /// 单槽时代这一格是"我那只手"（`Option<PieToken>`）；孔有队列之后，内核对"哪只是我的"
    /// 没有身份、只有**深度**——单推的一只孔上，深度就是我还压着的手数。故写端由"猜自己的手"
    /// 改成"问内核"（[`Sender::outstanding`]）。
    outstanding: usize,
    /// 最后推出去的是**自己那格 `buf`** 还是**借来的那段**——只决定 [`Drop`] 要不要收口。
    own: bool,
    _m: PhantomData<M>,
}

impl<M: Message> Sender<M> {
    /// 空格：**没有写端**（`send` 会答 SendFail::Unbound）。`const` 是给"放进结构体里当一格"
    /// 那些用到上的（`Guest` 那一格）
    pub const fn new() -> Self {
        Self {
            hole: None,
            buf: M::EMPTY,
            outstanding: 0,
            own: false,
            _m: PhantomData,
        }
    }

    /// 认下一枚**别人给的**号（服务端那一侧：孔是对方铸的、交给我的）
    pub fn from_token(hole: PieToken) -> Self {
        Self {
            hole: Some(hole),
            buf: M::EMPTY,
            outstanding: 0,
            own: false,
            _m: PhantomData,
        }
    }

    pub fn send(&mut self, msg: M) -> Result<(), SendFail> {
        let Some(hole) = self.hole else {
            return Err(SendFail::Unbound);
        };
        // **自己带一格缓冲 ⇒ 一次只能有一只手下线**：先把它收回（队列空），再编、再推。
        self.reclaim().map_err(|e| SendFail::Mail(e.source))?;
        let Some(n) = msg.store(self.buf.as_mut()) else {
            return Err(SendFail::TooLong);
        };
        let bytes = self.buf.as_ref().get(..n).ok_or(SendFail::TooLong)?;
        HolePie::from_token(hole)
            .push(bytes, Wait::POLL)
            .map_err(|e| SendFail::Mail(e.source))?;
        self.outstanding += 1;
        self.own = true;
        Ok(())
    }

    /// **借一段递出去**：字节留在**调用方**那儿（取走那一刻内核复制一次），故本层不复制。
    ///
    /// 与 [`Sender::send`] 的分界：那一档自己带一格缓冲（一次只能一只手）；这一档的字节是
    /// 调用方的 ⇒ **可以连推**，直到孔上那一列排满（答 `Mail(Busy)`）。
    /// **借出去那段的寿命归调用方**：内核取走之前它不能改、不能放（[`Drop`] 不为它收口）。
    pub fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), SendFail> {
        let Some(hole) = self.hole else {
            return Err(SendFail::Unbound);
        };
        HolePie::from_token(hole)
            .push(bytes, Wait::POLL)
            .map_err(|e| SendFail::Mail(e.source))?;
        self.outstanding += 1;
        self.own = false;
        Ok(())
    }

    /// 问内核：**我还排着几手**（一次 `Peek`，非阻塞），并把本层那个数刷新。
    pub fn outstanding(&mut self) -> Result<usize, SendFail> {
        let Some(hole) = self.hole else {
            return Err(SendFail::Unbound);
        };
        let depth = HolePie::from_token(hole)
            .depth()
            .map_err(|e| SendFail::Mail(e.source))?;
        self.outstanding = depth;
        Ok(depth)
    }

    /// 不碰内核的那个数（诊断：由上次 [`Sender::outstanding`]／[`Sender::send`] 得来）。
    pub const fn hint(&self) -> usize {
        self.outstanding
    }

    /// **等这只手下线**：送到（队列回到空）或孔封印（`Err(Dead)`）或孔不见了（`Gone`）
    /// **没有期限**——见文件头②。已经空着 ⇒ 当场 `Ok`（零代价）
    ///
    /// **队列化之后这一手等的仍是"空"**（不是"有位"）：单推的一只孔上，队列空 ⟺ 我推出去的
    /// 那几手全下线了。要"有位"用 [`Sender::send_bytes`] 那一档（满了答 `Busy`，不睡）。
    pub fn reclaim(&mut self) -> MailResult<()> {
        let Some(hole) = self.hole else {
            return Ok(());
        };
        if self.outstanding == 0 {
            return Ok(());
        }
        let r = HolePie::from_token(hole).wait(HoleDir::Push, Wait::Forever);
        if let Err(e) = &r {
            crate::debug!(
                "mail: reclaim miss hole={} code={}",
                hole.get(),
                e.source.code()
            );
        }
        self.outstanding = 0;
        r.map(|_| ())
    }

    /// **非阻塞收口**：我推出去的都下线了 ⇒ 放下那一格、答 `true`；还压着 ⇒ 答 `false`。
    /// 判据是**问内核的那一个数**（[`Sender::outstanding`]），不再是"单槽空不空"。
    pub fn settle(&mut self) -> bool {
        if self.outstanding == 0 {
            return true;
        }
        self.outstanding().map(|n| n == 0).unwrap_or(false)
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

/// **落出作用域 = 收口**：还压着**自己那格 `buf`** 就等它下线
/// 这是"递出即走"能安全成立的那一半（见文件头④）：`send` 不睡，代价是"这段字节还欠着"，而
/// 欠的那一段是自己的内存 ⇒ 走之前必须还清。
///
/// **借出去的那一段不等**（[`Sender::send_bytes`]）：它的寿命归调用方，内核取走之前是调用方
/// 的事——落出作用域就等一句"队列空"，会把写端挂在慢读者身上（树那一侧不能被订户挂住）。
impl<M: Message> Drop for Sender<M> {
    fn drop(&mut self) {
        if self.own && self.outstanding > 0 {
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
