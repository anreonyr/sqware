//! driver::context — **本域在系统里的位置**：一条会话 ＋ 本域那枚服务门牌。
//! ```text
//!   Context   Session（[`protocol::communication::session`]）＋ entry（本域的服务入口）
//!   join      上板（只为让板看得见本域的死）→ 开那条会话
//!   enter     起手那一趟：解门牌 → join → 上树落门牌 → 占线（`rtc` 那一台走）
//!   line      从树上找线路由者 ＋ 占住本域那条线
//!   publish   把一批字节推给本域服务门的客人
//! ```

use crate::driver::shared::fail::Fail;
use crate::service::operator::bridge;
use crate::unit::Died;
use env::{PieToken, TaskId, Wait};
use protocol::communication::session::Session;
use protocol::debug;
use protocol::driver::ENTRY_MARK;
use protocol::driver::line::client::Line;
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
    Tree,
}

impl Context {
    /// **入系统**：上板（只为让板看得见本域的死）→ 开树那条会话。
    /// 门牌由调用方**先**解（各域的失败格不同：两台的 `unseal` 折 `tree`，路由者折 `desk`）。
    pub fn join(entry: PieToken, sire: TaskId, ms: Wait) -> Result<Context, Step> {
        let session = Session::open(sire, operator::BERTH, ms).map_err(|_| Step::Tree)?;
        Ok(Context { entry, session })
    }

    /// **起手那一趟**（`rtc` 那一台走）：解门牌 → 上板 ＋ 开会话 → 上树 → 占线。
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

    /// **上树那一趟**：分目录 → 落门牌 → 查回来 → 按号问名。
    /// **这一趟本身住在 `bridge::land`**（`system/operator/bridge.rs`）：名册 / 盟册两处服务、本手、
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
    /// `line` 是**契里给的那个号**（本域不自己算它）；入口经会话从树上授进来，
    /// 泊位由 [`Line`] 那一层装。那一趟（译号带重试 ＋ 取那一枚）在
    /// [`operator::Face::tile`]，本手从它取门闩。
    /// **它拿一面借来的视图**而不是收走会话：`Context` 持着这条会话（`uart` 那一台还要从它
    /// 编自己那两枚门牌），故 [`operator::Face::from`] 按值取一份视图（树那三格是 `Copy`）。
    pub fn line(&self, line: u32, ms: Wait) -> Result<Line, ()> {
        // 路是**驱动那一族的常量**（`/svc/drv`）接上服务名——一处都不自己拼。
        let road = protocol::driver::ROAD.try_join(ROUTER).ok_or(());
        let Ok(road) = road else { return Err(()) };
        let tree = operator::Face::from(&self.session);
        // **这一手三条出口都折成调用方那一句 `line`** ⇒ 红的时候看不出是哪一格，故这里各印一行。
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
    /// **两半都写出来**（旧合成 `push` 就是这两半）：**等轮到自己** ＋ **等这只手被取走**。
    pub fn publish(&self, bytes: &[u8]) -> Result<(), ()> {
        let door = runtime::env::mail::HolePie::from_token(self.entry);
        door.push(bytes, Wait::Forever).map_err(|_| ())?;
        door.wait(env::HoleDir::Push, Wait::Forever)
            .map(|_| ())
            .map_err(|_| ())
    }
}
