//! principal::client — **客侧**：按门牌借一枚回信孔，一问一答。
//!
//! ```text
//!   Face::of(门牌)               门牌那一枚是树上查回来的（开者 = 对端）
//!   bind / resolve / derive /
//!   sire / heir                  一问一答：替这一趟铸一枚回信孔借过去，答完丢掉
//! ```
//!
//! **问话走门牌、答话走这一趟自带的那一枚孔**：报文里没有"往哪回"这一格——号只在持有它的
//! 那张表里念得动（`protocol::session` 事实 8），故每一趟借一枚新的回信孔过去，收的人按
//! "谁给的 + 记号"两格认出它，答完当场放下。
//!
//! **没有会话可选装**：这一面不装码头、不定泊位——门牌自己就是那条路（同 rtc 那一面）。

use contract::message::Message;
use env::Wait;
use env::{PieToken, TaskId};
use runtime::env::mail;

use super::call::{self, BACK};
use super::core::{Fail, PrincipalId};
use crate::session::slip::Slip;

pub use super::call::opened_by;

/// 一面身份服务：**树上查回来的门牌** + 它的开者（对端）。
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

    /// 名册 · 写：把一条 TID 定到一条已存在的号上。**只有装配者那一枚问得出 `OK`**。
    pub fn bind(&self, tid: TaskId, p: PrincipalId, millis: Wait) -> Result<(), Fail> {
        let out = self.raw(call::Req::Bind(tid, p), millis)?;
        self.answer(out, |_flag, _at| Ok(()))
    }

    /// 名册 · 读：这条 TID 此刻代表谁。`None` = 没绑（**不是失败**）。
    pub fn resolve(&self, tid: TaskId, millis: Wait) -> Result<Option<PrincipalId>, Fail> {
        let out = self.raw(call::Req::Resolve(tid), millis)?;
        self.answer(out, |flag, at| {
            Ok(flag.then(|| PrincipalId::new(at as usize)))
        })
    }

    /// 谱系 · 写：由 `p` 派生一枚新节点（装配者，或当前正好代表 `p` 的那一枚）。
    pub fn derive(&self, p: PrincipalId, millis: Wait) -> Result<PrincipalId, Fail> {
        let out = self.raw(call::Req::Derive(p), millis)?;
        self.answer(out, |_flag, at| Ok(PrincipalId::new(at as usize)))
    }

    /// 转换 · 领：把**自己**当前的号换成 `q`（只许沿自己那一支向下）。
    pub fn adopt(&self, q: PrincipalId, millis: Wait) -> Result<(), Fail> {
        let out = self.raw(call::Req::Adopt(q), millis)?;
        self.answer(out, |_flag, _at| Ok(()))
    }

    /// 转换 · 弃：回到**装配给我的那一条**（不删格，故还能再领一次）。
    pub fn waive(&self, millis: Wait) -> Result<(), Fail> {
        let out = self.raw(call::Req::Waive, millis)?;
        self.answer(out, |_flag, _at| Ok(()))
    }

    /// 谱系 · 读：直接父。**三态**——`Some` / `None`（它是根）/ `Err(Unknown)`（树外）。
    pub fn sire(&self, p: PrincipalId, millis: Wait) -> Result<Option<PrincipalId>, Fail> {
        let out = self.raw(call::Req::Sire(p), millis)?;
        self.answer(out, |flag, at| {
            Ok(flag.then(|| PrincipalId::new(at as usize)))
        })
    }

    /// 谱系 · 读：`a ≼ b`。
    pub fn heir(&self, a: PrincipalId, b: PrincipalId, millis: Wait) -> Result<bool, Fail> {
        let out = self.raw(call::Req::Heir(a, b), millis)?;
        // `HEIR` 的答案在**有没有**那一格（是 / 不是），8 字节那一格留空。
        self.answer(out, |flag, _at| Ok(flag))
    }

    /// 问一句、取一句答。
    ///
    /// **照实记（传输失败折进语义那一格）**：借不出回信孔 / 超时 / 答话长度不对——三件事都答
    /// [`Fail::Denied`]，与"**对端说了不**"同一格。压它的理由同 [`Face::answer`] 那一格：**对
    /// 本端是同一个下一步**（这一趟别指望了）。要给它单开一格，就得往 [`Fail`] 里加一个变体，
    /// 而那一份是 `fail_codes!` 的**双射表**（加变体 = 加一个线上码）——那是动协议面的事。
    /// 分得开它们的那一格在**对面**：`Denied` 是服务真会答的码（判据在 `judge`），
    /// "没走到"是本端自己在码表之外判的。
    fn raw(&self, act: call::Req, millis: Wait) -> Result<call::Reply, Fail> {
        // **先铸、先交，再推**（次序是契约的一半，见 `session::call::lend_out`）：那一枚
        // "种在对端表里的号"随帧一起过去 ⇒ 对端一次 `Reserve` 就认得出，不必扫自己的表。
        let (back, seed) =
            crate::session::call::lend_out(self.entry, BACK).map_err(|()| Fail::Denied)?;
        // 编一问：**一张表 ＋ 一处编**（`back` 是运输那一格，随动作一起进帧）。
        let mut frame = [0u8; call::Query::LEN];
        act.query(seed).store(&mut frame);
        if crate::session::call::push_to(self.entry, &frame).is_err() {
            // 推不出去 ⇒ 这一趟根本没到对端，那一枚收回来（与 `lend` 同一手收尾）。
            let _ = mail::release(back);
            return Err(Fail::Denied);
        }
        // 收：答话走**这一趟借出去的那一枚孔**（船台那一手；缓冲由调用方给——这一形 10 字节）。
        // 两格失败（没收到 / 解不动）在这一侧落同一格：`Denied`（对本端是同一个下一步）。
        let mut buf = call::Reply::EMPTY;
        let got = Slip::<call::Reply>::seal(back)
            .land(buf.as_mut(), millis)
            .map_err(|_| Fail::Denied);
        // 这一趟的回信孔只活到这句话答完：收走就放下（不管成没成）。
        let _ = mail::release(back);
        got
    }

    /// 一句答：先看状态那一格（失败域 + 读不懂），再看答案那一格。
    fn answer<T>(
        &self,
        reply: call::Reply,
        read: impl FnOnce(bool, u64) -> Result<T, Fail>,
    ) -> Result<T, Fail> {
        match call::code_to_fail(reply.status) {
            None if reply.status == call::OK => read(reply.flag, reply.a),
            Some(fail) => Err(fail),
            // **读不懂在这一侧与"没走到"同一格**（照实记）：对本端是同一个下一步——
            // 别指望这条路；Principal 那一侧说不出"我没接住"这句话（`BAD` 是 Server 说的，
            // 走到这里说明连它那一格都没读成）。
            None => Err(Fail::Denied),
        }
    }
}
