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
//! 树上那几手（"名字 → 号 → 入口"、落门牌）回它们自己的协议客手
//! （[`operator::entry_of`] / [`operator::plate`]）。本目录只剩**设备面那一半**：
//! 门牌、线、推一批字节。
//!
//! **两条判据**（与旧两份逐字同，正文现在住那两个定义处）：
//!
//! - **名字只到 `seek` 那一格**：此后一律按号，`find` / `name` 都收号；
//! - **失败即断言**（[`operator::plate`]）：`part` / `land` / `find` 任一非 `OK`、`got` 假、
//!   号 ↔ 名对不上 ⇒ 当场死。它不是一条错误分支，而是"这一域没登记上就不该活着"的判据。

use crate::driver::fail::Fail;
use crate::program::Died;
use env::{Key, Name, PieToken, TaskId, Wait};
use protocol::communication::session::Session;
use protocol::debug;
use protocol::driver::line::client::Line;
use protocol::system::board::ENTRY_MARK;
use protocol::system::board::client as board;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::Mine;
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
        operator::plate(&ctx.session, protocol::driver::DIR, me, mine, ctx.entry, ms);
        let line = ctx.line(key, ms).map_err(|_| Fail::at(died, "line"))?;
        debug!("{me}: line occupied");
        Ok((ctx, line))
    }

    /// **登记本域那一条线**：从树上找到线路由者（`/device/router`）＋ 占住。
    ///
    /// 坐标是**配给回给本域的那一段区**（本域不写死它）；入口经会话从树上授进来，
    /// 泊位由 [`Line`] 那一层装。那一趟（译号带重试 ＋ `find`）见 [`operator::entry_of`]。
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
