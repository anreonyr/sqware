//! session — **客人开局**：一条 `operator` 会话 ＋ 在树上按路找/落一枚入口。
//!
//! ```text
//!   Session::open   开会话（装上那条路 ＋ 要一枚问话孔）
//!   id_of           名字 → 号（**译不出就重试**——那几格可能由别的域落下，落得比本域晚）
//!   entry_of        号 → 入口
//!   lookup          id_of ＋ entry_of（带重试的那一趟）
//!   Session::service `lookup` 的 `/dir/name` 两面（失败折成一格）
//!   Session::land   在 `dir` 那一段之下落一枚门牌，并当场查回来核对（五条判据 ＋ 一行读数）
//! ```
//!
//! # 照实记（为什么住 crate 顶层，而不在 `driver::`）
//!
//! 这一半原先住 `driver::context`——那是"折叠残枝"那一刀的产物：`driver/tree.rs` 的"上树
//! 那一趟"、`driver/register.rs` 的"登记那一条线"与 `harness/lodger.rs` 的 `find_router`
//! 三份逐字同构，并成一条 `Session`。并完之后它的**用户里一半不是驱动**：
//!
//! ```text
//!   驱动    uart / rtc / router —— 上树落门牌、从树上找线路由者
//!   房客    harness/lodger     —— 没有门牌，只要"开会话 ＋ 找一枚入口"这一半
//!   客人    user/echo          —— 找 `/device/uart` 那扇门
//!   内件    system/coalition   —— 找 `/sys/…` 那一扇
//! ```
//!
//! 一个 `user` 档的程序写 `use programs::driver::context::Session` 是**名字越界**
//! （`driver/` 那一档讲的是设备面）。故这一刀把它抬到 crate 顶层：`driver::` 回到"设备面那一档
//! （驱动 ＋ 它们的装配件）"。"客人怎么开局"与本 crate 其余共享件同层。
//!
//! # 照实记（"带预算重试"那一份曾抄了八遍）
//!
//! [`id_of`] 那一圈重试原先是**八份**几乎逐字相同的复制：`echo/adapt/console.rs`、
//! `harness/sleeper.rs::find_face`、`harness/probe_rule_other.rs::denied`、
//! `system/coalition/server.rs::find_face`、`harness/subject.rs::find_face`、
//! `harness/member.rs::find_face`、`harness/probe_rule.rs::{find_face,seek_id}`、
//! `harness/guest.rs` 那一圈。而**预算口径还不一样**：多数把**剩下的**当这一趟的期限
//! （总账 ≤ `MS`），另有几份每趟都给满 `MS`（总账 = 重试次数 × `MS`——`echo` 那份的照实记
//! 早把后者记成缺陷）。合成一处之后统一取前者：[`Wait::AtMost(n)`](Wait) 里的 `n` 是
//! **总预算**、逐趟递减；`Wait::Forever` ⇒ 不限。
//!
//! **那一圈为什么存在**：门牌是**别的域**落的，本域可能比它先起（[`RETRY_MS`] 那一睡就是那一格）。
//! 健康机器上一趟就命中。

use core::time::Duration;
use env::{Name, PieToken, TaskId, Wait};
use protocol::communication::establish::Endpoint;
use protocol::debug;
use protocol::system::operator as ocall;
use protocol::system::operator::EntryId;
use protocol::system::operator::Where;
use protocol::system::operator::client as operator;
use runtime::env::room;

/// 名字译不出号时，再问之前睡多久（毫秒）。
const RETRY_MS: usize = 1;

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

    /// **名字 → 号 → 入口**：从树上按 `/dir/name` 那两段找到一扇门（失败折成一格）。
    ///
    /// **间接寻址那一手**：名字先译成号（号才是树的直接坐标），此后按号——那一条重试在
    /// [`lookup`] 里（"对面可能比本域晚落"）。
    pub fn service(&self, dir: &str, name: &str, ms: Wait) -> Result<PieToken, ()> {
        let (Ok(dir), Ok(name)) = (Name::new(dir), Name::new(name)) else {
            return Err(());
        };
        lookup(&self.link, self.talk, &[dir, name], ms).map_err(|_| ())
    }

    /// **上树那一趟**：分目录 → 落门牌 → 查回来 → 按号问名（五条判据 ＋ 一行读数）。
    ///
    /// `dir` 是本域那一族在树上的那一段（驱动族是 `protocol::driver::DIR`）；`me` 既是
    /// `LAND` / `FIND` 的那一段，也是读数前缀——三台是**同一个串**（服务名）。
    /// `entry` 是本域那枚服务入口。
    pub fn land(&self, dir: &str, me: &str, mine: Mine, entry: PieToken, ms: Wait) {
        let (Ok(dir), Ok(name)) = (Name::new(dir), Name::new(me)) else {
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

    /// 本端那条路的三样：`echo` 那台上树**探针**要自己调 `operator`（它量的是树上那几支判据，
    /// 不是第二份开局）。除它以外没有第二个调用者——开局一律走本文件那几手。
    pub fn parts(&self) -> (&Endpoint, PieToken, TaskId) {
        (&self.link, self.talk, self.host)
    }
}

/// 门牌那一格声不声明归属（`operator::land` 的最后一格）。
///
/// 三台驱动今天都是**公开可查**（`ocall::Rule::Public`），只在这一格上分家：`uart` 说"这枚
/// 读行的孔是我的"（[`Mine::Yes`]），`rtc` / `router` 不说（[`Mine::No`]）。
#[derive(Clone, Copy)]
pub enum Mine {
    /// 这一格是我的。
    Yes,
    /// 不声明归属。
    No,
}

/// 沿一条路译成号，**译不出（`UNKNOWN`）就重试**（见头注那一格）。
///
/// 总预算是 `ms`：每一趟把**剩下的**当期限递下去（不是每趟都给满——那样"预算"是假的）。
pub fn id_of(link: &Endpoint, talk: PieToken, road: &[Name], ms: Wait) -> Result<EntryId, u8> {
    let mut left = match ms {
        Wait::AtMost(n) => n,
        Wait::Forever => usize::MAX,
    };
    loop {
        match operator::seek(talk, link, road, Wait::AtMost(left)) {
            Ok(id) => return Ok(id),
            Err(ocall::UNKNOWN) if left > 0 => {
                let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
                left = left.saturating_sub(RETRY_MS);
            }
            Err(code) => return Err(code),
        }
    }
}

/// 号 → 入口：答话码原样答（`find` 那一族的失败折成 [`ocall::BAD`]）。
pub fn entry_of(link: &Endpoint, talk: PieToken, id: EntryId, ms: Wait) -> Result<PieToken, u8> {
    match operator::find(talk, link, id, ms) {
        Ok((ocall::OK, Some(entry))) => Ok(entry),
        Ok((code, _)) => Err(code),
        Err(_) => Err(ocall::BAD),
    }
}

/// [`id_of`] ＋ [`entry_of`]：**那一趟带重试的寻址**（八份复制并成这一处）。
pub fn lookup(link: &Endpoint, talk: PieToken, road: &[Name], ms: Wait) -> Result<PieToken, u8> {
    entry_of(link, talk, id_of(link, talk, road, ms)?, ms)
}
