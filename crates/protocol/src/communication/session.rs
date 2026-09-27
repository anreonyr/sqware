//! session — **一条会话**：装一条路 → 认对端 → 要一枚问话孔。
//!
//! ```text
//!   Berth    一条路的名字：泊位那一格（`link`）＋ 问话孔那一格（`ask`）——由各协议自己声明
//!   Session  装好的那条路：本端那一对孔（答话 / 问话）＋ **对端的号**
//!   open     装路 → 认下对端那一枚 → 收下"答话的是谁" → 铸问话孔交给它
//!   own      本端那一枚读孔已经铸好（也就自己知道对端号）→ 只铸问话孔交给它
//! ```
//!
//! # 照实记（它原先在两份客手里各写一遍）
//!
//! `board::client::{open, ask_hole, hear, me}` 与 `operator::client::{open, ask_hole, hear, me}`
//! **逐字同构**，只差两格记号（泊位 / 问话孔）与各自的失败域。故按"两台以上逐字同构 ⇒ 收"
//! 把那四手抬回**地板这一层**：本层只认孔与路（不认识服务与 RPC，见 [`super`]）。
//!
//! **本层因此多了一样东西**：一条**会话**。它仍是"关系怎么建立"那一件事（[`super`] 头一句），
//! 只是粒度从"一枚孔"升到"一条路"。**它有两手**——`open`（两边各铸一枚、互相认）与
//! `own`（本端就是装配者那一档：读孔自己铸、对端号自己知道）——三个字段直接公开：读的人要的
//! 就是那三样，中间不再架一层 `link()` / `talk()` / `host()` / `parts()` 那样的转发。
//!
//! # 两个记号为什么由调用方给
//!
//! 一条路叫什么（`operator` / `board`）是**那份协议自己的事实**（各协议 `frame.rs` 里那两格，
//! 经 `client.rs` 的 `BERTH` 交出来）；本层只按它把路装起来，不写死哪几条路。**问话孔按记号
//! 分人**，故两条路的 `ask` 不能同名——它们本来就不同（`operator-ask` / `board-ask`）。

use env::wire::Field;
use env::{Mark, PieToken, TaskId, Wait};
use runtime::env::mail;

use super::establish::{self, Endpoint};

/// 一条路的名字：**泊位那一格**（`link`）＋ **问话孔那一格**（`ask`）。
#[derive(Clone, Copy)]
pub struct Berth {
    /// 这条路（泊位）叫什么。
    pub link: Mark,
    /// 这条路上那枚问话孔的记号。
    pub ask: Mark,
}

/// 一条装好的会话：本端那一对孔（答话走 `link`、问话走 `talk`）＋ **对端的号**（`host`）。
pub struct Session {
    /// 本端**答话**那一枚（本端读）。
    pub link: Endpoint,
    /// **问话**那一枚（本端写、对端读）。
    pub talk: PieToken,
    /// **对端的号**（"答话的是谁"）。
    pub host: TaskId,
}

/// [`Session::open`] 失败在哪一格（两格各一个不同的下一步）。
pub enum Fail {
    /// 这条路没接上：装泊位 / 认对端那一枚 / 收"答话的是谁"——三步任一没成。
    Link,
    /// 问话孔铸不出来、或交不出去。
    Ask,
}

impl Session {
    /// 开一条到 `berth` 那条路的会话。
    ///
    /// `holder` = 客人认的对端 = **它的生我者**：孔交给它，它再转授给那条路上真正的服务
    /// ——故"服务是谁"得由装配者告诉（见各协议 `client.rs` 的头注）。
    pub fn open(holder: TaskId, berth: Berth, millis: Wait) -> Result<Session, Fail> {
        let link = establish::endpoint(holder, berth.link, millis).map_err(|_| Fail::Link)?;
        // **认不到对端那一枚 = 这条路没接上**：两侧各装一条、凑齐才算通。
        if link.tx().is_none() {
            return Err(Fail::Link);
        }
        let host = hear(&link, millis).ok_or(Fail::Link)?;
        let talk = ask(host, berth.ask).map_err(|_| Fail::Ask)?;
        Ok(Session { link, talk, host })
    }

    /// **本端那一侧已经装好了**：本端铸的那一枚读孔在手里，只需再铸一枚问话孔交给 `host`。
    ///
    /// [`Session::open`] 那一路是"两边各铸一枚、互相认"；本手给的是**本端就是装配者**那一档
    /// （编排域当自己那棵树的客人：那两个"认"的动作在本端表里会认到本域替别的客人垫的孔，
    /// 见 [`Endpoint::own`]）。故：
    ///
    /// - **读孔由调用方铸、由调用方交**（`rx` 是它自己刚铸的那一枚）——本手不扫表、也不铸；
    /// - **对端号由调用方给**（它本来就知道是谁）：`open` 那一步的 `hear` 是"问对面是谁"，
    ///   而这里问的人就是拿主意的人；
    /// - **问话孔照旧由本手铸**（[`ask`]）：那一条路上"一个域只铸一枚"的构造不变。
    pub fn own(rx: PieToken, berth: Berth, host: TaskId) -> Result<Session, Fail> {
        let talk = ask(host, berth.ask).map_err(|_| Fail::Ask)?;
        Ok(Session {
            link: Endpoint::own(rx),
            talk,
            host,
        })
    }
}

/// 收下路上那一格：**答话的是谁**（装配侧 `bridge.rs` 的 `tell` 的对偶）。
///
/// 宽度与字节序归 [`Field`](env::wire::Field) 给 [`TaskId`] 那一对 `store` / `fetch`
/// ——这一格从前在五处各写一遍（那一对里记着）。
///
/// 返 `None` = 期限到了还没到 ⇒ 这条服务没接上（客人报它自己的超时，不猜）。
fn hear(link: &Endpoint, millis: Wait) -> Option<TaskId> {
    let mut buf = [0u8; TaskId::WIDTH];
    match mail::HolePie::from_token(link.rx()).pull_timeout(&mut buf, millis) {
        Ok(n) if n == TaskId::WIDTH => TaskId::fetch(&buf),
        _ => None,
    }
}

/// 铸**问话孔**并交给对端（本端随即自窄到只写）。
///
/// **一个域一条路只铸一枚**——先找我表里那一枚，有就不铸第二枚。认的是"**本端开的** ＋ 记号"
/// 两格（[`establish::find`]，与对端认孔那两格正判据同一句话）。于是"只铸一枚"从**纪律**变成
/// **构造**：这条路上再也生不出第二枚，而第二枚的症状是"多出来的那枚永远没人读它的推"。
fn ask(host: TaskId, mark: Mark) -> Result<PieToken, ()> {
    if let Some(have) = establish::find(me(), mark) {
        return Ok(have);
    }
    // 铸 + 交出读端 + 本端窄到只写：一手就是 `establish::give`。
    establish::give(host, mark).map_err(|_| ())
}

/// **本端是哪一枚线程**——"这一枚孔是谁开的"那一问要它。
///
/// 不返 `Result`：`SelfId` 那一格恒写 id（生成的入口标了 `#[infallible]`）。
fn me() -> TaskId {
    runtime::env::unit::self_id()
}
