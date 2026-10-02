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

/// 要找的那位服务（线路由者）在树上的名字
const ROUTER: &str = "router";

pub struct Context {
    pub entry: PieToken,
    pub session: Session,
}

pub enum Step {
    /// 树那条会话（开会话 / 要问话孔）
    Tree,
}

impl Context {
    /// 门牌由调用方**先**解（各域的失败格不同：两台的 `unseal` 折 `tree`，路由者折 `desk`）
    pub fn join(entry: PieToken, sire: TaskId, ms: Wait) -> Result<Context, Step> {
        let session = Session::open(sire, operator::BERTH, ms).map_err(|_| Step::Tree)?;
        Ok(Context { entry, session })
    }

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

    /// **它拿一面借来的视图**而不是收走会话：`Context` 持着这条会话（`uart` 那一台还要从它
    /// 编自己那两枚门牌），故 operator::Face::from 按值取一份视图（树那三格是 `Copy`）
    pub fn line(&self, line: u32, ms: Wait) -> Result<Line, ()> {
        // 路是**驱动那一族的常量**（`/svc/drv`）接上服务名——一处都不自己拼。
        let road = protocol::driver::ROAD.try_join(ROUTER).ok_or(());
        let Ok(road) = road else { return Err(()) };
        let tree = operator::Face::from(&self.session);
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
            // 同一个 Fail::Denied，而那两个数就是这七格的钥匙——探子把它们印出来。
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

    /// **两半都写出来**（旧合成 `push` 就是这两半）：**等轮到自己** ＋ **等这只手被取走**
    ///
    /// **无期限**：只有"字节借的是自己的、且当场就没人取了"那几处才用得起它（服务台那种
    /// "一条线程服所有人"的地方**不能用**——见 [`Context::offer`] 那一节的注）。
    pub fn publish(&self, bytes: &[u8]) -> Result<(), ()> {
        let door = runtime::env::mail::HolePie::from_token(self.entry);
        door.push(bytes, Wait::Forever).map_err(|_| ())?;
        // **（临时读数）第二半**（`Wait::Forever`）：这一半是"等**收方**把我这一批取走"，而收方
        // 那一刻可能正卡在它自己那一手"等我把它的字取走"上 ⇒ 两边互等。
        //
        // **必须成对**：`begin` 在**动手之前**打、`done` 在**回来之后**打，并带同一个 `seq`——
        // 否则"等出去就再没回来"这一档看不见（只报"等了 ≥300 ms"是事后读数，卡死的那一趟
        // 恰恰永远不打）。有 `begin` 无 `done` = **它就停在这一句上**。两个都封顶 40。
        let seq = {
            static S: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
            S.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed)
        };
        if seq < 40 {
            debug::put(&alloc::format!(
                "console: publish wait begin tok={} me={} seq={}",
                self.entry.get(),
                utask::self_id().get(),
                seq
            ));
        }
        let t = runtime::env::chrono::clock();
        let r = door.wait(env::HoleDir::Push, Wait::Forever);
        let ms = (runtime::env::chrono::clock().saturating_sub(t) / 1_000_000) as usize;
        if seq < 40 {
            debug::put(&alloc::format!(
                "console: publish wait done seq={} ms={} ok={}",
                seq,
                ms,
                r.is_ok()
            ));
        }
        r.map(|_| ()).map_err(|_| ())
    }

    /// **交这一批的第一半（单次尝试）**：`true` = 已经递出去（**那段字节从此到被取走之前不能动、
    /// 不能放**）；`false` = 槽满（`Busy`）或那一格没了 ⇒ 这一批**还没递出去**，下趟再试。
    ///
    /// **为什么要有它**：`publish` 的两半都是无期限的，而"服务台"（一台设备、一条线程、一群客人）
    /// 的圈里**不能有无限期的等**——它是几路人马共用的那一枚线程，卡在一半上就把另几路一起卡住，
    /// 且对方等的往往正是"回到圈首去做的那件事"。故驱动那一侧走 [`Context::offer`] ＋
    /// [`Context::taken`] 两半：**凑不上就回去干别的，下圈再来**（字节住在调用方自己那一格里）。
    pub fn offer(&self, bytes: &[u8]) -> bool {
        let door = runtime::env::mail::HolePie::from_token(self.entry);
        match door.push(bytes, Wait::POLL) {
            Ok(()) => true,
            // 槽满（`Busy`）＝ 对方还没取走上一条：**不是错**，下趟再试。
            Err(e) if e.source.is_busy() => false,
            // 那一格没了（封了/拆了）：这一批送不到，调用方自己处置（见 [`Context::taken`]）。
            Err(_) => false,
        }
    }

    /// **交这一批的第二半（有期）**：`Some(true)` = **已经被取走**；`Some(false)` = 期限到了、
    /// 还没被取走（字节**必须留着**，下趟再来等）；`None` = 那一格没了 ⇒ 这一批再也送不到。
    pub fn taken(&self, within: Wait) -> Option<bool> {
        let door = runtime::env::mail::HolePie::from_token(self.entry);
        door.wait(env::HoleDir::Push, within).ok()
    }
}
