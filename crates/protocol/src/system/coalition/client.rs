//! coalition::client — **客侧**：按门牌借一枚回信孔，一问一答。
//!
//! ```text
//!   Face::of(门牌)            门牌那一枚是树上查回来的（开者 = 对端）
//!   found / enter /
//!   leave / amid              一问一答：替这一趟铸一枚回信孔借过去，答完丢掉
//!   band / bloc               同上，只是答话里带回**一窗号**（3 + 8n 字节）
//! ```
//!
//! **问话走门牌、答话走这一趟自带的那一枚孔**：报文里没有"往哪回"这一格——号只在持有它的
//! 那张表里念得动（`protocol::session` 事实 8），故每一趟借一枚新的回信孔过去，收的人按
//! "谁给的 + 记号"两格认出它，答完当场放下。
//!
//! **客侧没有"我是谁"这一格**：`enter` / `leave` 不带身份参数——对端拿的是内核盖的那枚印章。
//! 这一格的诚实性是**签名给的**，不是纪律给的（见正文"已知边界"）。
//!
//! **没有会话可选装**：这一面不装码头、不定泊位——门牌自己就是那条路（同 rtc / principal
//! 那两面）。

use contract::message::Message;
use env::Wait;
use env::{PieToken, TaskId};
use runtime::env::mail;

use super::frame::{self, BACK};
use super::core::{CoalitionId, Fail, Window};
use crate::id::Id;
use crate::session::slip::Slip;

pub use super::opened_by;

use crate::system::principal::core::PrincipalId;

/// 一面结盟服务：**树上查回来的门牌** + 它的开者（对端）。
pub struct Face {
    entry: PieToken,
    host: TaskId,
}

impl Face {
    /// 把一枚门牌收成一面。
    ///
    /// 对端从**这一枚门闩自己**问出来（[`opened_by`]）——门牌是 Server 挂的，不是本端开的。
    pub fn of(entry: PieToken) -> Result<Face, Fail> {
        let host = opened_by(entry).ok_or(Fail::Unknown)?;
        Ok(Face { entry, host })
    }

    /// 对端是谁（读数用）。
    pub fn host(&self) -> TaskId {
        self.host
    }

    /// 盟 · 写：立一枚新号——**自己是哪一位**由内核盖的印章说。
    pub fn found(&self, millis: Wait) -> Result<CoalitionId, Fail> {
        let said = self.ask(frame::Req::Found, millis)?;
        self.answer(said, |_flag, at| Ok(CoalitionId::new(at as usize)))
    }

    /// 盟 · 写：**我**进 `c`。
    pub fn enter(&self, c: CoalitionId, millis: Wait) -> Result<(), Fail> {
        let said = self.ask(frame::Req::Enter(c), millis)?;
        self.answer(said, |_flag, _at| Ok(()))
    }

    /// 盟 · 写：**我**出 `c`。撞空也成（集合运算没有"第二次"）。
    pub fn leave(&self, c: CoalitionId, millis: Wait) -> Result<(), Fail> {
        let said = self.ask(frame::Req::Leave(c), millis)?;
        self.answer(said, |_flag, _at| Ok(()))
    }

    /// 盟 · 读：`p` 在不在 `c` 里。**两件事两个落点**——`Ok(false)` 是不在，
    /// `Err(Unknown)` 是这枚盟不存在。
    pub fn amid(&self, p: PrincipalId, c: CoalitionId, millis: Wait) -> Result<bool, Fail> {
        let said = self.ask(frame::Req::Amid(p, c), millis)?;
        // `AMID` 的答案在**有没有**那一格（在 / 不在），8 字节那一格留空。
        self.answer(said, |flag, _at| Ok(flag))
    }

    /// 盟 · 读：`c` 里此刻有谁（**一趟取窗**）。
    ///
    /// 序 = 号序升序；`after` 是**阈值**（取号 > 它的那些），`None` = 从头取。窗装不下 ⇒
    /// `Window::more` 为真——接着取就把**末一枚**当下一趟的 `after`。
    pub fn band(
        &self,
        c: CoalitionId,
        after: Option<PrincipalId>,
        millis: Wait,
    ) -> Result<Window<PrincipalId>, Fail> {
        self.window(frame::Req::Band(c, after), millis)
    }

    /// 盟 · 读：`p` 此刻在哪些盟里（**一趟取窗**，序与游标同 [`Face::band`]）。
    pub fn bloc(
        &self,
        p: PrincipalId,
        after: Option<CoalitionId>,
        millis: Wait,
    ) -> Result<Window<CoalitionId>, Fail> {
        self.window(frame::Req::Bloc(p, after), millis)
    }

    /// 问一句、收一答（**形状由长度分**：这一族三形在线上分得开，见 [`frame::Union`]）。
    ///
    /// **照实记（传输失败折进语义那一格，同 `principal/client.rs` 那一面）**：借不出回信孔 /
    /// 超时 / 收不下一答（空帧、长度不落在三形里）——三件事都答 [`Fail::Unknown`]，与"**这枚盟没
    /// 铸过**"同一格。压它的理由与那一面同：**对本端是同一个下一步**（这一趟别指望了），而这一族
    /// **没有 `Denied`** 可落（盟无主）。要单开一格就得往 [`Fail`] 里加变体，那一份是
    /// `fail_codes!` 的**双射表**（加变体 = 加线上码）——较真值得，但它是动协议面的一刀，不混在
    /// 这一条里。
    fn ask(&self, act: frame::Req, millis: Wait) -> Result<frame::Union, Fail> {
        // **先铸、先交，再推**（同 `principal/client.rs` 那一面；身体在 `session::call::lend_out`）。
        let (back, seed) =
            crate::session::call::lend_out(self.entry, BACK).map_err(|()| Fail::Unknown)?;
        // 编一问：**一张表 ＋ 一处编**（`back` 是运输那一格，随动作一起进帧）。
        let mut frame = [0u8; frame::Query::LEN];
        act.query(seed).store(&mut frame);
        if crate::session::call::push_to(self.entry, &frame).is_err() {
            let _ = mail::release(back);
            return Err(Fail::Unknown);
        }
        // 收：答话走**这一趟借出去的那一枚孔**（船台那一手；缓冲由调用方给＝本族最大那一形）。
        // 两格失败（没收到 / 解不动）在这一侧落同一格：`Unknown`（对本端是同一个下一步）。
        let mut buf = frame::Union::EMPTY;
        let got = Slip::<frame::Union>::seal(back)
            .land(buf.as_mut(), millis)
            .map_err(|_| Fail::Unknown);
        // 这一趟的回信孔只活到这句话答完：收走就放下（不管成没成）。
        let _ = mail::release(back);
        got
    }

    /// 一句答（**一格答那一形**）：**不是那一形 ⇒ 读不懂**，是那一形再看状态那一格（失败域），
    /// 最后才是荷载。
    fn answer<T>(
        &self,
        rep: frame::Union,
        read: impl FnOnce(bool, u64) -> Result<T, Fail>,
    ) -> Result<T, Fail> {
        // **先判形状**（照抄从前那条 `n == REPLY_LEN`）：一格状态那一形（失败）在这一条路上读不出
        // `FULL`——它走"没走到"那一格，与从前同。
        let frame::Union::One(reply) = rep else {
            return Err(Fail::Unknown);
        };
        match frame::code_to_fail(reply.status) {
            None if reply.status == frame::OK => read(reply.flag, reply.a),
            Some(fail) => Err(fail),
            // **读不懂在这一侧与"没走到"同一格**（照实记）：对本端是同一个下一步——
            // 别指望这条路；本族没有 `Denied` 这一格可落（盟无主），故两件事都答 `Unknown`。
            None => Err(Fail::Unknown),
        }
    }

    /// 一句窗答：**先看状态那一格**（失败域 + 读不懂），再认窗那一形。
    ///
    /// **照实记（`ask_seq` 退场）**：收那一手从前分两支（`raw` 要恰好 10、`ask_seq` 收
    /// `≤ 3 + 8 × 16`），而两支的身体逐字同构（铸孔 → 交孔 → 推 → 收 → 放）。形状由长度分得开
    /// 之后，两者只差**认哪一形**，故收成一枚 [`Face::ask`]。
    ///
    /// **次序（照抄从前那条 `read_seq`）**：**先看码**——一格状态那一形在这里读得出 `FULL`（一路
    /// 走到 [`Fail::Full`]）；一格答那一形不是窗，它那一格码照样交出来。
    fn window<T: Id>(&self, act: frame::Req, millis: Wait) -> Result<Window<T>, Fail> {
        match self.ask(act, millis)? {
            frame::Union::Seq(seq) => Ok(seq.window()),
            frame::Union::Status(code) | frame::Union::One(frame::Reply { status: code, .. }) => {
                Err(frame::code_to_fail(code).unwrap_or(Fail::Unknown))
            }
        }
    }
}
