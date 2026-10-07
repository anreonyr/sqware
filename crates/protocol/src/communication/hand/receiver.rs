//! 这一路流的那一种报由类型参数说。
//! # 缓冲由**调用方**给（与 Sender 相反）
//! 发那一侧可以自己带：那条报是自己编的，超不出本族最长（M::Buf）。收这一侧不行——
//! **对面推得进来什么，孔不预设**（内核不再有"一条消息 ≤ 一页"那条界），而 M::Buf 只保证
//! "本族合法的最长那一枚"。比 M::Buf 长的那一条：核答 `Denied` 而**手原样**（丢一条消息
//! 不可逆）——拿 M::Buf 收就是读到一个"读不懂"，可那一条**还留在孔上** ⇒ 组每轮都唤醒、

use core::marker::PhantomData;

use env::{MailFail, PieToken, Wait};

use crate::wire::message::Message;
use ::resource::raw::{HolePie};

/// **我收的那一枚孔** ＋ 这一路流的那一种报（类型）
pub struct Receiver<M: Message> {
    hole: PieToken,
    _m: PhantomData<M>,
}

impl<M: Message> Receiver<M> {
    /// 认下一枚**别人给的**号（服务端那一侧：孔是对方铸的、交给我的）
    pub fn from_token(hole: PieToken) -> Self {
        Self {
            hole,
            _m: PhantomData,
        }
    }

    /// 收 ＋ 解。`wait` = 等多久（`POLL` = 只探测）
    /// 失败三格**分得开**（RecvFail）：搬不动（`Mail`）/ 收到了解不动（`Unread`）——
    /// 而 `Mail` 里 `Busy`（期限内没等到）与 `Dead` / `Denied`（这一枚孔用不动了）也分得开
    pub fn recv(&self, buffer: &mut [u8], wait: Wait) -> Result<M::In, RecvFail> {
        let (n, _from) = HolePie::from_token(self.hole)
            .pull(buffer, wait)
            .map_err(|e| RecvFail::Mail(e.source))?;
        let bytes = buffer.get(..n).ok_or(RecvFail::Unread(n))?;
        M::fetch(bytes).ok_or(RecvFail::Unread(n))
    }

    /// **队里还排着几手**（`Peek` 的第三格；空队答 **0**，不是错误）。
    ///
    /// 孔上可以排着至多 `QUEUE_CAP` 只手：读者据此知道"还有几条要取"（取干为止的那一圈
    /// 就是拿它当上界）。
    pub fn depth(&self) -> Result<usize, RecvFail> {
        HolePie::from_token(self.hole)
            .depth()
            .map_err(|e| RecvFail::Mail(e.source))
    }

    /// 这一枚孔（**挂进组**用：一台驱动要同时等"线上有投递"与"门上有人"）
    /// 与 Receiver::recv 读的是同一枚——组等的是**就绪**，取消息仍走 `recv`
    pub fn hole(&self) -> PieToken {
        self.hole
    }
}

/// 收不回来：三格**分得开**（与 `rack::RecvFail` 同一套词）
/// - RecvFail::Unread = **收到了、解不动**（长度不对 / 形状不对 / 缓冲比帧还短）
/// **带那一条读到了几字节**。**它不属于载体那一层**：搬字节的那一手不做解码，故"读不懂"在
#[derive(Debug)]
pub enum RecvFail {
    Mail(MailFail),
    /// 收到了 `len` 字节，解不动（`buffer` 前 `len` 字节就是那一条原始帧）
    Unread(usize),
}
