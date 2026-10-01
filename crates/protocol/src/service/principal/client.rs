//! principal::client — **客侧**：一面身份服务，以及它的两个柄（[`Task`] / [`Principal`]）。
//! ```text
//!   Face::of(门牌)           门牌那一枚是树上查回来的（开者 = 对端）
//!   Face::task(tid)          把一个线程收成 Task（读面）
//!   Face::principal(p)       把一条号收成 Principal（读面）
//!   Face::new_principal()    从根派生一条新号
//!   Task::principal / bind   这一条线程此刻代表谁 / 把它定到一条号上
//!   Principal::derive / adopt / waive / sire / contains   这一条号自己那几手
//! ```

use crate::wire::message::Message;
use env::Wait;
use env::{HoleDir, PieToken, TaskId};
use runtime::env::mail;

use super::frame::{self, BACK, Fail, PrincipalId};
use crate::communication::establish;
use crate::communication::receiver::{Receiver, RecvFail};

/// 一面身份服务：**树上查回来的门牌** + 它的开者（对端）。
pub struct Face {
    entry: PieToken,
    host: TaskId,
}

impl Face {
    /// 把一枚门牌收成一面。
    /// 对端从**这一枚门闩自己**问出来（[`establish::opened_by`]）——门牌是 Server 挂的，不是本端开的。
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        let host = establish::opened_by(entry).ok_or(Fail::Unknown)?;
        Ok(Face { entry, host })
    }

    /// 对端是谁。**这是这一面唯一的读数，不出线**（不在那七条原语里）。
    /// 全树无生产消费者，留着只为"这一面在跟谁说话"答得出来——它是诊断读数，不是协议面。
    pub fn host(&self) -> TaskId {
        self.host
    }

    /// 把一个线程收成 [`Task`]（读面：只固定那条 `TaskId`，不对端问一句）。
    pub fn task(&self, tid: TaskId) -> Task<'_> {
        Task { face: self, tid }
    }

    /// 把一条号收成 [`Principal`]（读面：号是别人给的标签，本手不去问它对不对）。
    pub fn principal(&self, p: PrincipalId) -> Principal<'_> {
        Principal { face: self, at: p }
    }

    /// `derive(ROOT)`：开一条**新**的身份（装配者那一枚才答应）。
    pub fn new_principal(&self) -> Principal<'_> {
        self.principal(PrincipalId::ROOT)
    }

    /// 问一句、取一句答。
    /// **传输失败折进 [`Fail::Denied`]**：借不出回信孔 / 超时 / 答话长度不对——三件事都答
    /// [`Fail::Denied`]，与"**对端说了不**"同一格。压它的理由：**对本端是同一个下一步**（这一趟
    /// 别指望了）；要给它单开一格，就得往 [`Fail`] 里加一个变体，而那一份是 `fail_codes!` 的
    /// **双射表**（加变体 = 加一个线上码）——那是动协议面的事。分得开它们的那一格在**对面**：
    /// `Denied` 是服务真会答的码，"没走到"是本端自己在码表之外判的。
    fn call(&self, act: frame::Req, wait: Wait) -> Result<frame::Reply, Fail> {
        fn deny(step: &str) -> Fail {
            crate::debug!("principal: call deny={step}");
            Fail::Denied
        }
        // **先铸、先交，再推**（次序是契约的一半，见 `communication::establish::lend_out`）：那一枚
        // "种在对端表里的号"随帧一起过去 ⇒ 对端一次 `Reserve` 就认得出，不必扫自己的表。
        let (back, seed) = establish::lend_out(self.entry, BACK).map_err(|()| deny("borrow"))?;
        // 编一问：**一张表 ＋ 一处编**（`back` 是运输那一格，随动作一起进帧）。
        let mut frame = [0u8; frame::Query::LEN];
        let n = act
            .query(seed)
            .store_at(&mut frame, 0)
            .ok_or_else(|| deny("encode"))?;
        let door = mail::HolePie::from_token(self.entry);
        if let Err(e) = door.push(&frame[..n], Wait::Forever) {
            crate::debug!("principal: call deny=push:{}", e.source.code());
            // 推不出去 ⇒ 这一趟根本没到对端，那一枚收回来。
            // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
            // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
            let _ = mail::seal(back);
            let _ = mail::release(back);
            return Err(Fail::Denied);
        }
        // 收：答话走**这一趟借出去的那一枚孔**（缓冲由调用方给——这一形 10 字节）。
        let mut buf = frame::Reply::EMPTY;
        let got = Receiver::<frame::Reply>::from_token(back)
            .recv(buf.as_mut(), wait)
            // 两格失败（没收到 / 解不动）在这一侧落同一格：`Denied`——**但哪一格要报得出来**。
            .map_err(|e| match e {
                RecvFail::Unread(len) => {
                    crate::debug!("principal: call deny=recv-unread len={len}");
                    Fail::Denied
                }
                RecvFail::Mail(m) => {
                    crate::debug!("principal: call deny=recv:{}", m.code());
                    Fail::Denied
                }
            });
        // 答话回来了 ⇒ 对面早取走了；没回来也得把这一手收口（那条报不许悬）：推的人等"孔空"。
        let _ = door.wait(HoleDir::Push, Wait::Forever);
        // 这一趟的回信孔只活到这句话答完：收走就放下（不管成没成）。
        // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
        // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
        let _ = mail::seal(back);
        let _ = mail::release(back);
        got
    }
}

/// **一条线程**：`TaskId` 是固定下来的宾语，`principal` / `bind` 都不再重复传它。
pub struct Task<'a> {
    face: &'a Face,
    tid: TaskId,
}

impl Task<'_> {
    /// 这一条线程是谁（读数用）。
    pub fn id(&self) -> TaskId {
        self.tid
    }

    /// 名册 · 读：这条线程**此刻**代表谁。`None` = 没绑（**不是失败**）；
    /// 这条号不在树里才是 `Err(Fail::Unknown)`。
    pub fn principal(&self, wait: Wait) -> Result<Option<Principal<'_>>, Fail> {
        let reply = self.face.call(frame::Req::Resolve(self.tid), wait)?;
        let (present, at) = decode(reply)?;
        Ok(present.then(|| self.face.principal(PrincipalId::new(at as usize))))
    }

    /// 名册 · 写：把这条线程定到一条已存在的号上（换绑 = 重定起点）。
    /// **只有装配者那一枚问得出 `OK`**。
    pub fn bind(&self, p: PrincipalId, wait: Wait) -> Result<(), Fail> {
        let reply = self.face.call(frame::Req::Bind(self.tid, p), wait)?;
        decode(reply).map(|_| ())
    }
}

/// **一条身份**：`PrincipalId` 是固定下来的宾语，那几条谱系 / 转换都不再重复传它。
pub struct Principal<'a> {
    face: &'a Face,
    at: PrincipalId,
}

impl Principal<'_> {
    /// 这一条号是几（读数用；**跨协议交接的只有这个值**）。
    pub fn id(&self) -> PrincipalId {
        self.at
    }

    /// 它是不是那条树根（零号是真格子，不是"没有"）。
    pub fn is_root(&self) -> bool {
        self.at == PrincipalId::ROOT
    }

    /// 谱系 · 写：由这一条派生一枚新节点（装配者，**或当前正好代表它**的那一枚）。
    pub fn derive(&self, wait: Wait) -> Result<Principal<'_>, Fail> {
        let reply = self.face.call(frame::Req::Derive(self.at), wait)?;
        let (_flag, at) = decode(reply)?;
        Ok(self.face.principal(PrincipalId::new(at as usize)))
    }

    /// 转换 · 领：把**自己**当前的号换成 `q`（只许沿自己那一支向下）。
    /// **报文里没有"我是谁"那一格**：它认的是内核盖的那枚印章，故宾语是"此刻的自己"，不是
    /// 这枚 Rust 值——柄不因此改变（`&self`）。
    pub fn adopt(&self, q: PrincipalId, wait: Wait) -> Result<(), Fail> {
        let reply = self.face.call(frame::Req::Adopt(q), wait)?;
        decode(reply).map(|_| ())
    }

    /// 转换 · 弃：回到**装配给我的那一条**（不删格，故还能再领一次）。
    pub fn waive(&self, wait: Wait) -> Result<(), Fail> {
        let reply = self.face.call(frame::Req::Waive, wait)?;
        decode(reply).map(|_| ())
    }

    pub fn drop(&self, wait: Wait) -> Result<(), Fail> {
        let reply = self.face.call(frame::Req::Drop, wait)?;
        decode(reply).map(|_| ())
    }

    /// 谱系 · 读：直接父。**三态**——`Some` / `None`（它是根）/ `Err(Unknown)`（树外）。
    pub fn sire(&self, wait: Wait) -> Result<Option<Principal<'_>>, Fail> {
        let reply = self.face.call(frame::Req::Sire(self.at), wait)?;
        let (present, at) = decode(reply)?;
        Ok(present.then(|| self.face.principal(PrincipalId::new(at as usize))))
    }

    /// 谱系 · 读：`p` 在**自己**这一支里吗——即原语那一形 `heir(p, self)` = `p ≼ self`。
    /// **方向是这一手最容易写反的一格**：线上那一问的两格是 `heir(a, b)` = `a ≼ b`（`a` 是
    /// `b` 的祖先，见 `programs/src/system/principal/core.rs` 的 `heir`），故问"`p` 是不是
    /// **自己**的祖先"要写 `principal(self).contains(p)`；反过来写就是另一个谓词。
    pub fn contains(&self, p: PrincipalId, wait: Wait) -> Result<bool, Fail> {
        let reply = self.face.call(frame::Req::Heir(p, self.at), wait)?;
        // `HEIR` 的答案在**有没有**那一格（是 / 不是），8 字节那一格留空。
        let (flag, _at) = decode(reply)?;
        Ok(flag)
    }
}

/// 一句答拆两格：先看状态那一格（失败域 + 读不懂），再交出 `(有没有, 号)` 那两格。
/// 它不用 `self`（纯解码）⇒ 自由函数，不是一个为了"看起来属于 Face"而写成方法的手。
fn decode(reply: frame::Reply) -> Result<(bool, u64), Fail> {
    match frame::code_to_fail(reply.status) {
        None if reply.status == frame::OK => Ok((reply.flag, reply.a)),
        Some(fail) => Err(fail),
        // 读不懂在这一侧与"没走到"同一格：对本端是同一个下一步。
        None => Err(Fail::Denied),
    }
}
