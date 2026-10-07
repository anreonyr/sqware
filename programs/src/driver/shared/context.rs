//! context —— 每台驱动都要走的那几步：**上树那条会话**、落门牌、占线。
//!
//! # 它只余会话（`entry` 那一格撤了）
//! 从前这里揣着一枚**入口孔**（`ENTRY_MARK`，"域自己那扇门"的老口径）。今天各台的门面各不
//! 相同——`uart` 落的是**两枚页**（一具架一枚，见 `driver/uart/adapt/desk.rs`）、`rtc`/`router`
//! 各自铸自己那一枚——故这一层只管**会话**（＋两句每条路都要做的小事：`plate` / `line`）。
//! 要落入口孔的台自己拿着那一枚（`main.rs`/`adapt/boot.rs` 里的局部），[`Context::plate`] 收它。
//!
//! # 一并撤掉的三手
//! `publish` / `offer` / `taken`：那是"一手一格"时代 uart 那两半握手（`Forever` 那一半与
//! `POLL` 那两半）的化身；两条路都改成一具架之后，谁也不等对方，故它们没有读者了。

use env::{PieToken, TaskId, Wait};
use ipc::session::Session;
use protocol::debug;
use protocol::driver::line::Line;
use protocol::system::control::Scope;
use protocol::system::operator::Permit;
use protocol::system::operator::client as operator;

/// 要找的那位服务（线路由者）在树上的名字
const ROUTER: &str = "router";

pub struct Context {
    pub session: Session,
}

pub enum Step {
    /// 树那条会话（开会话 / 要问话孔）
    Tree,
}

impl Context {
    /// **只开会话**：入口孔不再是这一层的事（域自己那一枚由它自己铸、自己落）。
    pub fn open(sire: TaskId, ms: Wait) -> Result<Context, Step> {
        let session = Session::open(sire, operator::BERTH, ms).map_err(|_| Step::Tree)?;
        Ok(Context { session })
    }

    /// 上树落**一枚门牌**（`entry` = 这一域自己铸的那一枚孔）。
    pub fn plate(&self, entry: PieToken, me: &str, ms: Wait) {
        let client = protocol::system::control::publication::Client::injected()
            .expect("driver: publication entry");
        let target = protocol::system::control::publication::Target::Service {
            scope: Scope::Driver,
            group: "".into(),
            name: me.into(),
        };
        client
            .publish(target, entry, Permit::Public, ms)
            .expect("driver: publication");
    }

    /// **它拿一面借来的视图**而不是收走会话：`Context` 持着这条会话（`uart` 那一台还要从它
    /// 编自己那两面），故 operator::Face::from 按值取一份视图（树那三格是 `Copy`）
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
}
