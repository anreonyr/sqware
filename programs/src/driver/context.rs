//! driver::context — **本域在系统里的位置**：一条 `operator` 会话 ＋ 本域那枚服务门牌。
//!
//! ```text
//!   Session   一条 operator 会话（link / talk / host）—— 一个域只开一条
//!   Context   Session ＋ entry（本域的服务入口）
//! ```
//!
//! **照实记（三处复制并成一处）**：本文件并掉的两份旧文件（`driver/tree.rs` 的"上树那一趟"、
//! `driver/register.rs` 的"登记那一条线"）与 `harness/lodger.rs` 里的 `find_router` 抄的是
//! **同一趟路**：开会话 → 要问话孔 → 名字译成号（`seek`）→ 按号取入口（`find`）。
//! 三处的参数表分别是 7 参 / 4 参 / 0 参，而它们要的只是**同一条会话**。
//!
//! **两条判据**（与旧两份逐字同）：
//!
//! - **名字只到 `seek` 那一格**：此后一律按号，`find` / `name` 都收号；
//! - **失败即断言**（[`Session::plate`]）：`part` / `land` / `find` 任一非 `OK`、`got` 假、
//!   号 ↔ 名对不上 ⇒ 当场死。它不是一条错误分支，而是"这一域没登记上就不该活着"的判据。
//!
//! **`lodger`（房客）只用 [`Session`]**：它没有服务门牌（不上树），但"从树上找到线路由者"
//! 那一趟与驱动逐字同构——故 [`Session::service`] 单独拿得出来。

use env::{Key, Name, PieToken, TaskId, Wait};
use protocol::communication::establish::Endpoint;
use protocol::debug;
use protocol::driver::line::client::Line;
use protocol::system::board::client as board;
use protocol::system::operator as ocall;
use protocol::system::operator::Where;
use protocol::system::operator::client as operator;

/// 要找的那位服务（线路由者）在树上的名字。
const ROUTER: &str = "router";

/// 门牌那一格声不声明归属（`operator::land` 的最后一格）。
///
/// 三台今天都是**公开可查**（`ocall::Rule::Public`），只在这一格上分家：`uart` 说"这枚读行的
/// 孔是我的"（[`Mine::Yes`]），`rtc` / `router` 不说（[`Mine::No`]）。
#[derive(Clone, Copy)]
pub enum Mine {
    /// 这一格是我的。
    Yes,
    /// 不声明归属。
    No,
}

/// **一条 `operator` 会话**：本域与持树者之间那条路。
///
/// 一个域只开一条（`operator::open` 装的是"一条叫 `operator` 的泊位"，同一个域开第二条会撞
/// 同名；而两次 `open` 拿到的是两条*不同*的会话，孔各归各的表，混用更糟——实测栽过）。
pub struct Session {
    link: Endpoint,
    talk: PieToken,
    host: TaskId,
}

impl Session {
    /// 开会话：装上那条路，并要一枚问话孔。
    pub fn open(sire: TaskId, ms: Wait) -> Result<Session, ()> {
        let (link, host) = operator::open(sire, ms).map_err(|_| ())?;
        let talk = operator::ask_hole(host).map_err(|_| ())?;
        Ok(Session { link, talk, host })
    }

    /// **上树那一趟**：分目录 → 落门牌 → 查回来 → 按号问名（五条判据 ＋ 一行读数）。
    ///
    /// `me` 既是 `LAND` / `FIND` 的那一段，也是读数前缀——三台是**同一个串**（服务名）。
    /// `entry` 是本域那枚服务入口。
    pub fn plate(&self, me: &str, mine: Mine, entry: PieToken, ms: Wait) {
        let (Ok(dir), Ok(name)) = (Name::new(protocol::driver::DIR), Name::new(me)) else {
            debug!("{me}: tree: bad name");
            return;
        };
        // **分目录 → 落门牌 → 查回来验一遍**：分与落各自**答出那一格的号**（"号出门"那一手）。
        // **分目录**：`part` 是**幂等**的——那块目录已经在就答它那个号（里面有没有东西不管）。
        let dir_at = operator::part(self.talk, &self.link, Where::Root, dir, ms);
        let (part, dir_id) = match dir_at {
            Ok(id) => (ocall::OK, id.get()),
            Err(code) => (code, 0),
        };
        // **落门牌**：答的是门牌自己那一格的号。
        let plate = match dir_at {
            Ok(at) => operator::land(
                self.talk,
                &self.link,
                self.host,
                Where::At(at),
                name,
                entry,
                ocall::Rule::Public,
                matches!(mine, Mine::Yes),
                ms,
            ),
            Err(code) => Err(code),
        };
        let (land, pid) = match plate {
            Ok(id) => (ocall::OK, id.get()),
            Err(code) => (code, 0),
        };
        // 查回来验一遍：**按号**（名字只在上面那两格用过，此后一律按号）。
        let (find, got) = match plate {
            Ok(id) => match operator::find(self.talk, &self.link, id, ms) {
                Ok((code, entry)) => (code, entry.is_some()),
                Err(_) => (ocall::BAD, false),
            },
            Err(code) => (code, false),
        };
        // 拿号问名：**号 ↔ 名**这一对对得起来，才算那枚号是真坐标。
        let pname = plate
            .ok()
            .and_then(|id| operator::name(self.talk, &self.link, id, ms).ok());
        debug!(
            "{me}: tree part={part} dir={dir_id} land={land} find={find} got={got} entry={} plate={pid} pname={}",
            entry.get(),
            pname.as_ref().map(|n| n.as_str()).unwrap_or("-"),
        );
        // **这一趟的判据**（值那几格从门那边搬进来：门只剩"这一行还在不在"）。
        {
            assert_eq!(part, ocall::OK)
        }
        assert_eq!(land, ocall::OK);
        {
            assert_eq!(find, ocall::OK)
        }
        assert!(got);
        assert_eq!(pname.as_ref().map(|n| n.as_str()), Some(me))
    }

    /// **名字 → 号 → 入口**：从树上按 `/dir/name` 那两段找到一扇门。
    ///
    /// **间接寻址那一手**：名字先译成号（号才是树的直接坐标），此后按号。
    pub fn service(&self, dir: &str, name: &str, ms: Wait) -> Result<PieToken, ()> {
        let dir = Name::new(dir).map_err(|_| ())?;
        let want = Name::new(name).map_err(|_| ())?;
        let road = [dir, want];
        let id = operator::seek(self.talk, &self.link, &road, ms).map_err(|_| ())?;
        match operator::find(self.talk, &self.link, id, ms) {
            // **查不到**与**授不出去**都落进这一格（`find` 的状态那一格说得出是哪一种）。
            Ok((ocall::OK, Some(entry))) => Ok(entry),
            _ => Err(()),
        }
    }

    /// 从树上找到线路由者（`/device/router`），把本域那一条线登记下来。
    ///
    /// 坐标是**配给回给本域的那一段区**（本域不写死它）；入口经会话从树上授进来，
    /// 泊位由 [`Line`] 那一层装。
    pub fn register(&self, key: Key, ms: Wait) -> Result<Line, ()> {
        let entry = self.service(protocol::driver::DIR, ROUTER, ms)?;
        Line::occupy(entry, key, ms).map_err(|_| ())
    }
}

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
        // 上板：**只为让板看得见本域的死**；不挂牌子——名字在树上。**问话孔照交**：不交的那一位
        // 在板账上永远"没挂齐"，板线程会一直退化成 1 ms 节拍（`board::settle` 的 `unarmed`）。
        let (_link, board) = board::open(sire, ms).map_err(|_| Step::Board)?;
        if board::ask_hole(board).is_err() {
            return Err(Step::Board);
        }
        let session = Session::open(sire, ms).map_err(|_| Step::Tree)?;
        Ok(Context { entry, session })
    }

    /// **上树那一趟**（三台逐字同构）——见 [`Session::plate`]。
    pub fn plate(&self, me: &str, mine: Mine, ms: Wait) {
        self.session.plate(me, mine, self.entry, ms);
    }

    /// **登记本域那一条线**——见 [`Session::register`]。
    pub fn line(&self, key: Key, ms: Wait) -> Result<Line, ()> {
        self.session.register(key, ms)
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
