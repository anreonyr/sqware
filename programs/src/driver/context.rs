//! driver::context — **本域在系统里的位置**：一条会话 ＋ 本域那枚服务门牌。
//!
//! ```text
//!   Context   Session（[`protocol::communication::session`]）＋ entry（本域的服务入口）
//!   join      上板（只为让板看得见本域的死）→ 开那条会话
//!   enter     起手那一趟：解门牌 → join → 上树落门牌 → 占线（`rtc` 那一台走）
//!   line      从树上找线路由者 ＋ 占住本域那条线
//!   publish   把一批字节推给本域服务门的客人
//! ```
//!
//! # 照实记（这一层只剩"设备面"）
//!
//! 这条会话与它那几手原先住这里（"折叠残枝"那一刀把 `driver/tree.rs`、`driver/register.rs`
//! 与 `harness/lodger.rs` 的 `find_router` 三份逐字同构并成一条 `Session`）。而它的**用户里
//! 一半不是驱动**——房客、客人、内件都要"开会话 ＋ 找一枚入口"⇒ 开门那一手（装路 / 认对端 /
//! 要问话孔）抬进 [`protocol::communication::session`]（那是**地板**：只认孔与路），
//! 树上那几手（"名字 → 号 → 入口"）回它们自己的协议客手
//! （[`operator::entry_of`]）；**上树那一趟**（`plate`）原也住那边，按"一个组合动作只有一个
//! 实现消费者就不强升为协议"的裁定**下移到这里**（[`Context::plate`]，task-2 那一刀）。
//! 本目录只剩**设备面那一半**：门牌、线、推一批字节。
//!
//! **两条判据**（与旧两份逐字同，正文现在住那两个定义处）：
//!
//! - **名字只到 `seek` 那一格**：此后一律按号，`find` / `name` 都收号；
//! - **失败即断言**（[`Context::plate`]）：`part` / `land` / `find` 任一非 `OK`、`got` 假、
//!   号 ↔ 名对不上 ⇒ 当场死。它不是一条错误分支，而是"这一域没登记上就不该活着"的判据。

use crate::driver::fail::Fail;
use crate::program::Died;
use env::{Key, Name, PieToken, TaskId, Wait};
use protocol::communication::session::Session;
use protocol::debug;
use protocol::driver::line::client::Line;
use protocol::system::board::ENTRY_MARK;
use crate::system::board::client as board;
use protocol::system::operator as ocall;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::Mine;
use protocol::system::operator::{Rule, Where};
use runtime::env::mail;
use runtime::env::unit as utask;

/// 要找的那位服务（线路由者）在树上的名字。
const ROUTER: &str = "router";

/// **本域在系统里的位置**：门牌（本域的服务入口）＋ 一条会话。
pub struct Context {
    /// 本域那枚服务入口（`board::ENTRY_MARK` 解出来的那枚孔）。
    pub entry: PieToken,
    /// 本域那**一条** `operator` 会话。
    pub session: Session,
}

/// [`Context::join`] 的失败格：**死在哪一步**（两格各一个不同的下一步）。
pub enum Step {
    /// 上板那两步（开板路 / 要问话孔）。
    Board,
    /// 树那条会话（开会话 / 要问话孔）。
    Tree,
}

impl Context {
    /// **入系统**：上板（只为让板看得见本域的死）→ 开树那条会话。
    ///
    /// 门牌由调用方**先**解（各域的失败格不同：两台的 `unseal` 折 `tree`，路由者折 `desk`）。
    /// 次序照旧：**板在前、树在后**（单故障读数与从前逐字相同；两件同时不成才可能换格子）。
    pub fn join(entry: PieToken, sire: TaskId, ms: Wait) -> Result<Context, Step> {
        // 上板：**只为让板看得见本域的死**；不挂牌子——名字在树上。**问话孔照交**（开会话那一手
        // 一并铸）：不交的那一位在板账上永远"没挂齐"，板线程会一直退化成 1 ms 节拍
        // （`board::settle` 的 `unarmed`）。
        let _board = Session::open(sire, board::BERTH, ms).map_err(|_| Step::Board)?;
        let session = Session::open(sire, operator::BERTH, ms).map_err(|_| Step::Tree)?;
        Ok(Context { entry, session })
    }

    /// **起手那一趟**（`rtc` 那一台走）：解门牌 → 上板 ＋ 开会话 → 上树 → 占线。
    ///
    /// **照实记（为什么只剩一台走它）**：`uart` 那一台是**双向**的（控制台有读口与写口），
    /// 它那一格因此是一块 Pane、两枚门牌，故走自己那一趟（`driver/uart/desk.rs::start`）；
    /// 本手这一形（一枚门牌）今天只有 `rtc` 用。
    ///
    /// **设备那一手在调用之前**（`uart` 开 `IER.RX`、`rtc` 读两次钟自证）：闸门归设备持有者，
    /// 而占线当场把线接上——次序倒过来，"线接上了、设备那侧还没开闸"那一瞬里的字节就没有
    /// 中断可等。故本手只收**入系统**那一半。
    ///
    /// **失败读数说步名**（`board` / `tree` / `line`）：域名由号带——`died` 就是装配表里
    /// "这一台死了"那一号（[`crate::program::uart::E_UART`] 那种），与内核出口印的同一个。
    /// **成功那一行读数仍带域名**（`debug!("{me}: line occupied")`）。
    pub fn enter(
        key: Key,
        me: &str,
        mine: Mine,
        died: Died,
        ms: Wait,
    ) -> Result<(Context, Line), Fail> {
        let entry = mail::unseal_hole(ENTRY_MARK).map_err(|_| Fail::at(died, "tree"))?;
        let ctx = Context::join(entry, utask::sire(), ms).map_err(|s| {
            Fail::at(
                died,
                match s {
                    Step::Board => "board",
                    Step::Tree => "tree",
                },
            )
        })?;
        ctx.plate(me, mine, ms);
        let line = ctx.line(key, ms).map_err(|_| Fail::at(died, "line"))?;
        debug!("{me}: line occupied");
        Ok((ctx, line))
    }

    /// **上树那一趟**：分目录 → 落门牌 → 查回来 → 按号问名（五条判据 ＋ 一行读数）。
    ///
    /// **照实记（这一手从协议层下移到这里，task-2 那一刀）**：它原先住
    /// `protocol::system::operator::client::plate`。它是**驱动族那一段路的装配 recipe**——
    /// `dir` 恒为 [`protocol::driver::DIR`]、`entry` 恒为 `Context::entry`，而两个调用点
    /// （[`Context::enter`] 与 `router` 的起手）手里都是 `Context` ⇒ 按"一个组合动作只有一个
    /// 实现消费者就不强升为协议"的裁定搬回这里，落成 `Context` 自己那一手。判据一个字没改。
    ///
    /// **下移改的是住处，不是调什么**：它仍调协议层那四手自由函数（[`operator::part`] /
    /// [`operator::land`] / [`operator::find`] / [`operator::name`]）——`Context` 只持
    /// `&self.session`（它还要留给 [`Context::line`] 与 `uart` 那一台），拿不出 `Face` 要的
    /// 所有权（`Face::of` 吃一条 `Session`）。
    pub fn plate(&self, me: &str, mine: Mine, ms: Wait) {
        let session = &self.session;
        let entry = self.entry;
        let (Ok(dir), Ok(who)) = (Name::new(protocol::driver::DIR), Name::new(me)) else {
            debug!("{me}: tree: bad name");
            return;
        };
        // **分目录 → 落门牌 → 查回来验一遍**：分与落各自**答出那一格的号**（"号出门"那一手）。
        // **分目录**：`part` 是**幂等**的——那块目录已经在就答它那个号（里面有没有东西不管）。
        let dir_at = operator::part(session.talk, &session.link, Where::Root, dir, ms);
        let (part, dir_id) = match dir_at {
            Ok(id) => (ocall::OK, id.get()),
            Err(code) => (code, 0),
        };
        // **落门牌**：答的是门牌自己那一格的号。
        let landed = match dir_at {
            Ok(at) => operator::land(
                session.talk,
                &session.link,
                session.host,
                Where::At(at),
                who,
                entry,
                Rule::Public,
                matches!(mine, Mine::Yes),
                ms,
            ),
            Err(code) => Err(code),
        };
        let (laid, pid) = match landed {
            Ok(id) => (ocall::OK, id.get()),
            Err(code) => (code, 0),
        };
        // 查回来验一遍：**按号**（名字只在上面那两格用过，此后一律按号）。
        let (found, got) = match landed {
            Ok(id) => match operator::find(session.talk, &session.link, id, ms) {
                Ok((code, entry)) => (code, entry.is_some()),
                Err(_) => (ocall::BAD, false),
            },
            Err(code) => (code, false),
        };
        // 拿号问名：**号 ↔ 名**这一对对得起来，才算那枚号是真坐标。
        let pname = landed
            .ok()
            .and_then(|id| operator::name(session.talk, &session.link, id, ms).ok());
        debug!(
            "{me}: tree part={part} dir={dir_id} land={laid} find={found} got={got} entry={} plate={pid} pname={}",
            entry.get(),
            pname.as_ref().map(|n| n.as_str()).unwrap_or("-"),
        );
        // **这一趟的判据**（值那几格从门那边搬进来：门只剩"这一行还在不在"）。
        {
            assert_eq!(part, ocall::OK)
        }
        assert_eq!(laid, ocall::OK);
        {
            assert_eq!(found, ocall::OK)
        }
        assert!(got);
        assert_eq!(pname.as_ref().map(|n| n.as_str()), Some(me))
    }

    /// **登记本域那一条线**：从树上找到线路由者（`/device/router`）＋ 占住。
    ///
    /// 坐标是**配给回给本域的那一段区**（本域不写死它）；入口经会话从树上授进来，
    /// 泊位由 [`Line`] 那一层装。那一趟（译号带重试 ＋ `find`）见 [`operator::entry_of`]。
    ///
    /// **照实记（这一处为什么留自由函数）**：`Context` **持**一条 `Session`，但它是这一族
    /// 共用的载体（`uart` 那一台要从 `ctx.session` 取 `talk` / `link` / `host` 编自己那两枚
    /// 门牌，见 `driver/uart/desk.rs`）⇒ 不能把它交给 `Face`（[`operator::Face::of`] 吃所有权，
    /// 见 `protocol::system::operator::client`）。本手只借 `&self.session`，故照旧走自由函数
    /// ——同一份判据，只是不经那一层壳。
    pub fn line(&self, key: Key, ms: Wait) -> Result<Line, ()> {
        let (Ok(dir), Ok(router)) = (Name::new(protocol::driver::DIR), Name::new(ROUTER)) else {
            return Err(());
        };
        let entry = operator::entry_of(&self.session, &[dir, router], ms).map_err(|_| ())?;
        Line::occupy(entry, key, ms).map_err(|_| ())
    }

    /// **把一批字节推给本域服务门的客人**（设备持有者那一侧的服务面）。
    ///
    /// 单次尝试：槽满 / 口封 ⇒ `Err`（调用方按自己的读数处置）。
    pub fn publish(&self, bytes: &[u8]) -> Result<(), ()> {
        runtime::env::mail::HolePie::from_token(self.entry)
            .push(bytes)
            .map_err(|_| ())
    }
}
