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

use env::{PieToken, TaskId};
use runtime::env::mail::{self, HolePie};

use super::call::{self, BACK};
use super::core::{Fail, PolicyId};

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
    pub fn bind(&self, tid: TaskId, p: PolicyId, millis: usize) -> Result<(), Fail> {
        let out = self.raw(call::BIND, tid.get() as u64, p.get() as u64, millis)?;
        self.answer(out, |_present, _at| Ok(()))
    }

    /// 名册 · 读：这条 TID 此刻代表谁。`None` = 没绑（**不是失败**）。
    pub fn resolve(&self, tid: TaskId, millis: usize) -> Result<Option<PolicyId>, Fail> {
        let out = self.raw(call::RESOLVE, tid.get() as u64, 0, millis)?;
        self.answer(out, |present, at| {
            Ok((present == 1).then(|| PolicyId::new(at as usize)))
        })
    }

    /// 谱系 · 写：由 `p` 派生一枚新节点（装配者，或当前正好代表 `p` 的那一枚）。
    pub fn derive(&self, p: PolicyId, millis: usize) -> Result<PolicyId, Fail> {
        let out = self.raw(call::DERIVE, p.get() as u64, 0, millis)?;
        self.answer(out, |_present, at| Ok(PolicyId::new(at as usize)))
    }

    /// 谱系 · 读：直接父。**三态**——`Some` / `None`（它是根）/ `Err(Unknown)`（树外）。
    pub fn sire(&self, p: PolicyId, millis: usize) -> Result<Option<PolicyId>, Fail> {
        let out = self.raw(call::SIRE, p.get() as u64, 0, millis)?;
        self.answer(out, |present, at| {
            Ok((present == 1).then(|| PolicyId::new(at as usize)))
        })
    }

    /// 谱系 · 读：`a ≼ b`。
    pub fn heir(&self, a: PolicyId, b: PolicyId, millis: usize) -> Result<bool, Fail> {
        let out = self.raw(call::HEIR, a.get() as u64, b.get() as u64, millis)?;
        // `HEIR` 的答案在**有没有**那一格（是 / 不是），8 字节那一格留空。
        self.answer(out, |present, _at| Ok(present == 1))
    }

    /// 问一句、取一句答。
    fn raw(&self, op: u8, a: u64, b: u64, millis: usize) -> Result<[u8; call::REPLY_LEN], Fail> {
        let frame = call::pack_ask(op, a, b);
        let back =
            crate::session::call::lend(self.entry, BACK, &frame).map_err(|()| Fail::Denied)?;
        let mut buf = [0u8; call::REPLY_LEN];
        let got = match HolePie::from_token(back).pull_timeout(&mut buf, millis) {
            Ok(n) if n == call::REPLY_LEN => Ok(buf),
            _ => Err(Fail::Denied),
        };
        // 这一趟的回信孔只活到这句话答完：收走就放下（不管成没成）。
        let _ = mail::release(back);
        got
    }

    /// 一句答：先看状态那一格（失败域 + 读不懂），再看答案那一格。
    fn answer<T>(
        &self,
        out: [u8; call::REPLY_LEN],
        read: impl FnOnce(u8, u64) -> Result<T, Fail>,
    ) -> Result<T, Fail> {
        let Some((status, present, at)) = call::unpack_reply(&out) else {
            return Err(Fail::Denied);
        };
        match call::code_to_fail(status) {
            None if status == call::OK => read(present, at),
            Some(fail) => Err(fail),
            // **读不懂在这一侧与"没走到"同一格**（照实记）：对本端是同一个下一步——
            // 别指望这条路；Principal 那一侧说不出"我没接住"这句话（`BAD` 是 Server 说的，
            // 走到这里说明连它那一格都没读成）。
            None => Err(Fail::Denied),
        }
    }
}
