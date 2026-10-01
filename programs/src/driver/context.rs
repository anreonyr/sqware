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
//! （[`operator::Face::tile`]）；**上树那一趟**（`plate`）原也住那边，按"一个组合动作只有一个
//! 实现消费者就不强升为协议"的裁定**先下移到这里**（[`Context::plate`]），而**这一刀又抬进
//! [`bridge::land`]**——量出四个消费者（三台驱动 ＋ 三台系统域各一处，设备账那一刀之后又多了
//! 一族：它逐类落 `/dev/<类>/<名>`），那条裁定不成立了。
//! 本目录只剩**设备面那一半**：门牌、线、推一批字节。
//!
//! **两条判据**（与旧两份逐字同，正文现在住那两个定义处）：
//!
//! - **名字只到 `seek` 那一格**：此后一律按号，`find` / `name` 都收号；
//! - **失败即断言**（[`Context::plate`]）：路那一趟空表、`land` / `find` 任一非 `OK`、
//!   号 ↔ 名对不上 ⇒ 当场死。它不是一条错误分支，而是"这一域没登记上就不该活着"的判据。
//!   （`got` 不再单列：它就是 `find` 成没成。）

use crate::driver::fail::Fail;
use crate::program::Died;
use crate::system::operator::bridge;
use env::{PieToken, TaskId, Wait};
use protocol::communication::session::Session;
use protocol::debug;
use protocol::driver::line::client::Line;
use protocol::driver::ENTRY_MARK;
use protocol::service::operator::Permit;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Mine;
use runtime::env::mail;
use runtime::env::unit as utask;

/// 要找的那位服务（线路由者）在树上的名字。
const ROUTER: &str = "router";

/// **本域在系统里的位置**：门牌（本域的服务入口）＋ 一条会话。
pub struct Context {
    /// 本域那枚服务入口（`protocol::driver::ENTRY_MARK` 解出来的那枚孔）。
    pub entry: PieToken,
    /// 本域那**一条** `operator` 会话。
    pub session: Session,
}

/// [`Context::join`] 的失败格：**死在哪一步**（两格各一个不同的下一步）。
pub enum Step {
    /// 树那条会话（开会话 / 要问话孔）。
    ///
    /// **照实记（`Board` 那一格退场）**：它从前是第一位——"上板那两步"。撤板那一刀把客侧那一条
    /// 会话整片撤了 ⇒ 这一格退场。
    Tree,
}

impl Context {
    /// **入系统**：上板（只为让板看得见本域的死）→ 开树那条会话。
    ///
    /// 门牌由调用方**先**解（各域的失败格不同：两台的 `unseal` 折 `tree`，路由者折 `desk`）。
    /// 次序照旧：**板在前、树在后**（单故障读数与从前逐字相同；两件同时不成才可能换格子）。
    pub fn join(entry: PieToken, sire: TaskId, ms: Wait) -> Result<Context, Step> {
        // **照实记（"上板"那一格退场：撤板那一刀）**：本域从前开一条 `board::BERTH` 会话，只为让
        // 板看得见它的死；板那一族的死信号整片退场（监督那一趟改读内核那一格）⇒ 这一格退场。
        let session = Session::open(sire, operator::BERTH, ms).map_err(|_| Step::Tree)?;
        Ok(Context { entry, session })
    }

    /// **起手那一趟**（`rtc` 那一台走）：解门牌 → 上板 ＋ 开会话 → 上树 → 占线。
    ///
    /// **`line` 那一格是**哪条线**（照实记：从前收的是坐标 `Key`）**——那个数来自本域刚认领
    /// 那一台的契（[`Deed`](protocol::driver::hub::Deed)），区→线那条权威在设备账那一台。
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
        line: u32,
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
                    Step::Tree => "tree",
                },
            )
        })?;
        ctx.plate(me, mine, ms);
        let held = ctx.line(line, ms).map_err(|_| Fail::at(died, "line"))?;
        debug!("{me}: line occupied");
        Ok((ctx, held))
    }

    /// **上树那一趟**：分目录 → 落门牌 → 查回来 → 按号问名（**四条判据** ＋ 一行读数）。
    ///
    /// **这一趟本身住在 `bridge::land`**（`system/operator/bridge.rs`）：名册 / 盟册两处服务、本手、
    /// `uart::desk::plate` 四处逐字同构，量出来的行数见它的照实记。本手只剩两件**本族的事实**：
    /// 路是**一条常量** [`protocol::driver::ROAD`]（`/svc/drv`，三段那一刀之后仍是两段），
    /// 砖的名字就是本域那一段（`me`），以及末尾那几条**判据**。
    ///
    /// **照实记（这一格栽过：把砖的名字也写成了路的一段）**：路是**容器链**，不含那一枚自己的
    /// 名字——本域那枚砖就叫 `me`，直接在 `/svc/drv` 底下。收这一趟时写成了 `&[DIR, me]` ⇒ 树先
    /// 就地立出一块**名叫 `me` 的 Pane**，再把砖落在**它底下**。实测那两格：
    /// `part at=/device name=router id=18`、`land at=18 name=router id=19`。后果不止一处：
    /// 所有按 `/svc/drv/router` 找**砖**的客人全落空（`lodger: no router`、`sleeper: no rtc plate`、
    /// `guest` 那一条 `find` 读到"不是砖"），`uart` 的 [`Context::line`] 也答 `line`
    /// （`exit tid=10 reason=0x9`）；uart 一死，`probe-owner` 那条"主人**还活着**"的前提跟着塌
    /// （它的 land 于是"该拒而没拒"）。**一处路的写法，五台程序的读数一起变。**
    ///
    /// **照实记（那条"一个消费者"的裁定不成立了）**：本手原按"一个组合动作只有一个实现消费者
    /// 就不强升为协议"留在这里；这一刀量出**四个消费者**（三台驱动 ＋ 三台系统域各一处），故它
    /// 抬进树那一族的词表旁（`operator::bridge`），与持树者那侧的 `Tree::plate` 成对。
    ///
    /// **照实记（判据从六条变四条）**：原来 `part` / `laid` / `found` / `got` / `pname` 各断言一次
    /// （`part` 那两条还各套了一对多余的花括号）；其中 `got` 与 `found` 是**同一条事实的两个说法**
    /// （`found` 成 ⇒ `got` 真，原码就是从同一个 `match` 里同时得出的）⇒ 收成 `find` 一条。
    /// "路那一趟"由**返回空表**说（`land` 的返值）：断在哪一段由它印的那一行读数说。
    ///
    /// **它借一面 `Face` 而不是收走会话**：`Context` 还要把这条会话留给 [`Context::line`] 与
    /// `uart` 那一台 ⇒ [`operator::Face::from`] 按值取一份视图（树那三格是 `Copy`）。
    pub fn plate(&self, me: &str, mine: Mine, ms: Wait) {
        let tree = operator::Face::from(&self.session);
        let list = [(me, self.entry)];
        let plated = bridge::land(
            &tree,
            me,
            &protocol::driver::ROAD,
            mine,
            Permit::Unset,
            &list,
            ms,
        );
        // **这一域的判据**：失败是一条"不该活着"，不是一条错误分支（值那几格从门那边搬进来：
        // 门只剩"这一行还在不在"）。
        assert_eq!(plated.len(), 1, "{me}: tree: road");
        let one = &plated[0];
        assert!(one.land.is_ok(), "{me}: tree: land");
        assert!(one.find.is_ok(), "{me}: tree: find");
        assert_eq!(
            one.named.as_ref().map(|name| name.as_str()),
            Some(me),
            "{me}: tree: name"
        );
    }

    /// **登记本域那一条线**：从树上找到线路由者（`/svc/drv/router`）＋ 占住。
    ///
    /// `line` 是**契里给的那个号**（本域不自己算它）；入口经会话从树上授进来，
    /// 泊位由 [`Line`] 那一层装。那一趟（译号带重试 ＋ 取那一枚）在
    /// [`operator::Face::tile`]，本手从它取门闩。
    ///
    /// **它拿一面借来的视图**而不是收走会话：`Context` 持着这条会话（`uart` 那一台还要从它
    /// 编自己那两枚门牌），故 [`operator::Face::from`] 按值取一份视图（树那三格是 `Copy`）。
    pub fn line(&self, line: u32, ms: Wait) -> Result<Line, ()> {
        // 路是**驱动那一族的常量**（`/svc/drv`）接上服务名——一处都不自己拼。
        let road = protocol::driver::ROAD.try_join(ROUTER).ok_or(());
        let Ok(road) = road else { return Err(()) };
        let tree = operator::Face::from(&self.session);
        // **这一手三条出口都折成调用方那一句 `line`** ⇒ 红的时候看不出是哪一格，故这里各印一行。
        // **照实记（它量到的那条抖，红率与定位都在）**：`rtc` ＋ `line` 那一红（手工 release、喂
        // `exit`）实测 **5 跑 4 红**（另一轮 **4 跑 3 红**），而每一次都落在**第三格**
        // （`Line::occupy`，往下见它自己记的 `cause`）：`cause=5`——**"请它占线"那一句推不进那扇门**
        // （`out.send(Occupy)` 失败；`deny(4,0)` 那一格"递回信孔"与 `deny(2)/(7)` 都过得了）。
        // **照实记（认错过一次）**：`bdb6125` 的告词把 `cause=5` 说成了"递回信孔被拒"——那是
        // `deny(4,0)`；两个块挨着，认错了一格，已在下游那一处改正。
        // 故这一条读数**留着**（它一响就写明是哪一格；查清之后随那一刀退场）。
        let entry = match tree.tile(&road, ms) {
            Ok(entry) => entry,
            Err(fail) => {
                debug::put(&alloc::format!("line: tile failed {fail:?}"));
                return Err(());
            }
        };
        let entry = match entry.token(ms) {
            Ok(entry) => entry,
            Err(fail) => {
                debug::put(&alloc::format!("line: token failed {fail:?}"));
                return Err(());
            }
        };
        let held = Line::occupy(entry, line, ms);
        if held.is_err() {
            // **那一手自己记了"死在哪一格"**（`deny(cause, code)` 两个静态）：它把七个出口折成
            // 同一个 `Fail::Denied`，而那两个数就是这七格的钥匙——探子把它们印出来。
            use core::sync::atomic::Ordering;
            use protocol::driver::line::client::{OCCUPY_CODE, OCCUPY_DENY};
            debug::put(&alloc::format!(
                "line: occupy failed line={line} entry={} cause={} code={}",
                entry.get(),
                OCCUPY_DENY.load(Ordering::Relaxed),
                OCCUPY_CODE.load(Ordering::Relaxed),
            ));
        }
        held.map_err(|_| ())
    }

    /// **把一批字节推给本域服务门的客人**（设备持有者那一侧的服务面）。
    ///
    /// **两半都写出来**（旧合成 `push` 就是这两半）：**等轮到自己** ＋ **等这只手被取走**。
    /// **照实记（"单次尝试"这一句是假的，已改真）**：本注从前写着"单次尝试：槽满 / 口封 ⇒ `Err`"，
    /// 而代码一直是合成那一手（递出 ＋ 等它下线）——**那一等不是多余的**：`bytes` 是**调用方
    /// 那一帧**的字节（`uart` 那个批），不等它下线就返回，客人复制到的就是一片被复用的栈。
    /// 故这一手保留"不等手取走不返回"，只把两半写明白。
    pub fn publish(&self, bytes: &[u8]) -> Result<(), ()> {
        let door = runtime::env::mail::HolePie::from_token(self.entry);
        door.push(bytes, Wait::Forever).map_err(|_| ())?;
        door.wait(env::HoleDir::Push, Wait::Forever)
            .map(|_| ())
            .map_err(|_| ())
    }
}
