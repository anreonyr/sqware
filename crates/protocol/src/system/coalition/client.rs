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

use env::{PieToken, TaskId};
use runtime::env::mail::{self, HolePie};

use super::call::{self, BACK};
use super::core::{CoalitionId, Fail, Window};
use crate::id::Id;

pub use super::call::opened_by;

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
    pub fn found(&self, millis: usize) -> Result<CoalitionId, Fail> {
        let out = self.raw(call::FOUND, 0, 0, millis)?;
        self.answer(out, |_present, at| Ok(CoalitionId::new(at as usize)))
    }

    /// 盟 · 写：**我**进 `c`。
    pub fn enter(&self, c: CoalitionId, millis: usize) -> Result<(), Fail> {
        let out = self.raw(call::ENTER, c.get() as u64, 0, millis)?;
        self.answer(out, |_present, _at| Ok(()))
    }

    /// 盟 · 写：**我**出 `c`。撞空也成（集合运算没有"第二次"）。
    pub fn leave(&self, c: CoalitionId, millis: usize) -> Result<(), Fail> {
        let out = self.raw(call::LEAVE, c.get() as u64, 0, millis)?;
        self.answer(out, |_present, _at| Ok(()))
    }

    /// 盟 · 读：`p` 在不在 `c` 里。**两件事两个落点**——`Ok(false)` 是不在，
    /// `Err(Unknown)` 是这枚盟不存在。
    pub fn amid(&self, p: PrincipalId, c: CoalitionId, millis: usize) -> Result<bool, Fail> {
        let out = self.raw(call::AMID, p.get() as u64, c.get() as u64, millis)?;
        // `AMID` 的答案在**有没有**那一格（在 / 不在），8 字节那一格留空。
        self.answer(out, |present, _at| Ok(present == 1))
    }

    /// 盟 · 读：`c` 里此刻有谁（**一趟取窗**）。
    ///
    /// 序 = 号序升序；`after` 是**阈值**（取号 > 它的那些），`None` = 从头取。窗装不下 ⇒
    /// `Window::more` 为真——接着取就把**末一枚**当下一趟的 `after`。
    pub fn band(
        &self,
        c: CoalitionId,
        after: Option<PrincipalId>,
        millis: usize,
    ) -> Result<Window<PrincipalId>, Fail> {
        self.window(call::BAND, c.get() as u64, call::cursor_of(after), millis)
    }

    /// 盟 · 读：`p` 此刻在哪些盟里（**一趟取窗**，序与游标同 [`Face::band`]）。
    pub fn bloc(
        &self,
        p: PrincipalId,
        after: Option<CoalitionId>,
        millis: usize,
    ) -> Result<Window<CoalitionId>, Fail> {
        self.window(call::BLOC, p.get() as u64, call::cursor_of(after), millis)
    }

    /// 问一句、取一句答。
    ///
    /// **照实记（传输失败折进语义那一格，同 `principal/client.rs` 那一面）**：借不出回信孔 /
    /// 超时 / 答话长度不对——三件事都答 [`Fail::Unknown`]，与"**这枚盟没铸过**"同一格。压它的
    /// 理由与那一面同：**对本端是同一个下一步**（这一趟别指望了），而这一族**没有 `Denied`**
    /// 可落（盟无主）。要单开一格就得往 [`Fail`] 里加变体，那一份是 `fail_codes!` 的**双射表**
    /// （加变体 = 加线上码）——较真值得，但它是动协议面的一刀，不混在这一条里。
    fn raw(&self, op: u8, a: u64, b: u64, millis: usize) -> Result<[u8; call::REPLY_LEN], Fail> {
        // **先铸、先交，再推**（同 `principal/client.rs` 那一面；身体在 `session::call::lend_out`）。
        let (back, seed) =
            crate::session::call::lend_out(self.entry, BACK).map_err(|()| Fail::Unknown)?;
        let frame = call::pack_ask(op, a, b, seed);
        if crate::session::call::push_to(self.entry, &frame).is_err() {
            let _ = mail::release(back);
            return Err(Fail::Unknown);
        }
        let mut buf = [0u8; call::REPLY_LEN];
        let got = match HolePie::from_token(back).pull_timeout(&mut buf, millis) {
            Ok(n) if n == call::REPLY_LEN => Ok(buf),
            _ => Err(Fail::Unknown),
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
            return Err(Fail::Unknown);
        };
        match call::code_to_fail(status) {
            None if status == call::OK => read(present, at),
            Some(fail) => Err(fail),
            // **读不懂在这一侧与"没走到"同一格**（照实记）：对本端是同一个下一步——
            // 别指望这条路；本族没有 `Denied` 这一格可落（盟无主），故两件事都答 `Unknown`。
            None => Err(Fail::Unknown),
        }
    }

    /// 问一句、取一句答（**窗那一档**：答话有长有短——窗是 `3 + 8n`，失败只有一格状态）。
    fn ask_seq(
        &self,
        op: u8,
        a: u64,
        b: u64,
        millis: usize,
    ) -> Result<([u8; call::SEQ_REPLY_LEN], usize), Fail> {
        // **先铸、先交，再推**（同本文件 `raw` 那一支）。
        let (back, seed) =
            crate::session::call::lend_out(self.entry, BACK).map_err(|()| Fail::Unknown)?;
        let frame = call::pack_ask(op, a, b, seed);
        if crate::session::call::push_to(self.entry, &frame).is_err() {
            let _ = mail::release(back);
            return Err(Fail::Unknown);
        }
        let mut buf = [0u8; call::SEQ_REPLY_LEN];
        let got = match HolePie::from_token(back).pull_timeout(&mut buf, millis) {
            Ok(n) if n <= call::SEQ_REPLY_LEN => Ok((buf, n)),
            _ => Err(Fail::Unknown),
        };
        // 这一趟的回信孔只活到这句话答完：收走就放下（不管成没成）。
        let _ = mail::release(back);
        got
    }

    /// 一句窗答：先看状态那一格（失败域 + 读不懂），再把那一串号读出来。
    fn window<T: Id>(&self, op: u8, a: u64, b: u64, millis: usize) -> Result<Window<T>, Fail> {
        let (out, n) = self.ask_seq(op, a, b, millis)?;
        match call::read_seq::<T>(&out[..n]) {
            Ok(window) => Ok(window),
            Err(code) => Err(call::code_to_fail(code).unwrap_or(Fail::Unknown)),
        }
    }
}
