//! 协调帧两格）请它做的两件事——接一位客人上树（Tree::attach）与递一条路上去
//! （`programs/src/system/operator/plate.rs`）；land 是**自己落**——各域开一条树会话、逐段
//! 分路、按名字把门牌贴上去、再查回来验一遍。两侧共用一个坐标（`["sys","principal"]` 那种路）。
//! **客人只有别的域**：装配者替每一位客人把孔转给持树者、再把它的号推上提示路。而"往树上立
//! 那一格的权威，而它当不了自己的客人（自指 ⇒ 环）。

use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

use env::Mark;
use env::Wait;
use env::wire::Field;
use env::{HoleDir, PieToken, TaskId};
use runtime::core::res::port::{self, Access, Policy};
use runtime::env::mail;

use crate::system::Assembly;
use crate::system::control::{BOOT_MS, Service};
use crate::unit::UnitFile;

use protocol::common::path::{Path, PathBuf};
use protocol::communication::session::establish;
use protocol::debug;
use protocol::service::operator::client::{Face, Mine, Pane, Watch};
use protocol::service::operator::{EntryId, Fail, Grant, Permit, Rule, TIP_LEN, Tip};
pub use protocol::service::operator::{LINK, TIP_MARK};

/// **只走提示之路**：那条路上三形各带一格 `kind`（读者是持树者，它按首格认形状）
fn push(into: PieToken, tip: Tip) -> Result<(), ()> {
    let mut rec = [0u8; TIP_LEN];
    let n = tip.store(&mut rec).ok_or(())?;
    let road = mail::HolePie::from_token(into);
    road.push(&rec[..n], Wait::Forever).map_err(|_| ())?;
    road.wait(HoleDir::Push, Wait::Forever).map_err(|_| ())?;
    Ok(())
}

/// **把一个号推过去**（`TaskId`，8 字节小端）——**树路上那一格**：告客人"答话的是谁"
pub(crate) fn tell(who: TaskId, into: PieToken) -> Result<(), ()> {
    let mut rec = [0u8; TaskId::WIDTH];
    who.store(&mut rec);
    let road = mail::HolePie::from_token(into);
    road.push(&rec, Wait::Forever).map_err(|_| ())?;
    road.wait(HoleDir::Push, Wait::Forever).map_err(|_| ())?;
    Ok(())
}

/// **持树者在装配者这一侧的状态**：持树者的号 ＋ 它那条提示之路
/// 它们问的是**树的语义**——客人怎么接、提示怎么认——故收进树这一间
#[derive(Default)]
pub struct Tree {
    host: Option<TaskId>,
    tip: Option<PieToken>,
}

impl Tree {
    /// 持树者那一枚的号（`None` = 还没起）
    pub fn host(&self) -> Option<TaskId> {
        self.host
    }

    /// **把这位客人接上树**（三步见 attach）。持树者还没起就没得接
    pub fn attach(&mut self, client: TaskId, millis: Wait) -> Result<(), &'static str> {
        let host = self.host.ok_or("no tree yet")?;
        attach(client, host, millis, &mut self.tip)
    }

    pub fn wire(&mut self) -> Result<(), &'static str> {
        let Some(tip) = self.tip else {
            return Err("no tip");
        };
        push(tip, Tip::Wired).map_err(|()| "operator:wire")
    }

    /// **它就是持树者本身**：认下它那条提示之路，此后客人上树才有路可走
    pub fn adopt(&mut self, host: TaskId, millis: Wait) -> Result<(), &'static str> {
        self.host = Some(host);
        self.tip = None;
        host_of(host, millis, &mut self.tip)?;
        Ok(())
    }

    /// **这一台是不是持树者**——判据是**它自己交出来的那一件东西**：提示之路上那枚挂在它名下的
    /// `TIP_MARK` 孔（establish::find **只看**，不另铸一枚新的）
    pub fn holds(&self, host: TaskId) -> bool {
        establish::find(host, TIP_MARK).is_some()
    }

    /// **递一条路上去**：请持树者把这条路上的窗格逐段立出来（缺的就地造），末段按 `leaf`
    /// 落叶子，或立窗格（`leaf = None` ⇒ 末段是**窗格**）
    /// 路是**绝对坐标**（从根起数）：`["sys","control"]`、`["sys","operator"]`、
    /// `["sys","operator","part"]` 三种落法**同一个形状**——连"父底下立一块窗格"、再深一层
    pub fn plate(
        &mut self,
        road: &Path,
        leaf: Option<PieToken>,
        rule: Rule,
    ) -> Result<(), &'static str> {
        let host = self.host.ok_or("no tree yet")?;
        let leaf = match leaf {
            // 有叶子：那一枚先交过去——`R|W ＋ VEST`（持树者要把它再授给来查的客人；少 `VEST`
            // ⇒ 客人那次 `find` 里的转授答 `Denied`）。**号随帧走**——本仓那条"号随交接一起走"。
            Some(entry) => port::ship(
                &mail::HolePie::from_token(entry),
                host,
                Access::FETCH | Access::STORE,
                Policy::VEST,
            )
            .map_err(|_| "operator:plate")?
            .seed(),
            // 末段是窗格：**没有可交的东西**（目录不是叶子——没有入口、没有 Pie，故不递孔；
            // 而递一枚没人立的孔只是让持树者表里多一枚死副本）。
            None => PieToken::NONE,
        };
        let tip = self.tip.ok_or("no tip")?;
        push(
            tip,
            Tip::Plate {
                road: road.to_path_buf(),
                leaf,
                rule,
            },
        )
        .map_err(|()| "operator:plate")
    }
}

pub fn attach_client(
    assembly: &mut Assembly,
    program: &UnitFile,
    service: &mut Service,
) -> Result<(), &'static str> {
    if !needs_tree(program) {
        return Ok(());
    }
    assembly.tree.attach(service.0, Wait::AtMost(BOOT_MS))
}

fn needs_tree(program: &UnitFile) -> bool {
    program
        .relation
        .after
        .is_some_and(|deps| deps.contains(&crate::unit::operator::PROGRAM.name()))
}

pub fn hold(
    assembly: &mut Assembly,
    _program: &UnitFile,
    service: &mut Service,
) -> Result<(), &'static str> {
    if !assembly.tree.holds(service.0) {
        return Ok(());
    }
    assembly.tree.adopt(service.0, Wait::AtMost(BOOT_MS))?;
    assembly.mount_grants();
    Ok(())
}

/// 把持树者接上一位客人（装配者调用）：**三步**
/// `host` = 持树者的号（service::spawn 交回来的那个，装配者本来就知道它）
/// `tip` = 提示之路在**本线程表里**的那一枚（第一次用时认下来，此后逐条传下去）
/// 返 `Err(哪一步)`：名字非法 / 席位满 / 等不到客人那一枚 / 提示孔认不到……对调用方是
/// 同一件事——**这条服务没接上树**——但"死在哪一步"正是装配诊断要的那一格
pub fn attach(
    client: TaskId,
    host: TaskId,
    millis: Wait,
    tip: &mut Option<PieToken>,
) -> Result<(), &'static str> {
    //      答话从客人自己那枚走）＋ 认领**这位客人**交出来的那一枚（记号 = 这条路的名字）。
    //      判据两格（`owner == client` ＋ 记号）与原 `seat` ＋ `claim` 逐字同源。
    let link = establish::endpoint(client, Mark::of(LINK), millis).map_err(|_| "operator:seat")?;
    // **认不到对端那一枚 = 这条路没接上**。
    if link.tx().is_none() {
        return Err("operator:claim");
    }
    // 3. 提示孔（只认一次）→ 把客人那一枚转授给持树者 → 告两边。
    //    要推的正是它（**不是** `host`：那是持树者的号，推不动）。
    let _ = host_of(host, millis, tip)?;
    let reply = link.tx().ok_or("operator:hand")?;
    hand(reply, host).map_err(|()| "operator:hand")?;
    // 客人那一侧的一格：**答话的是谁**（持树者的号，8 字节**裸号**——那条路的读者是
    // communication::session::hear，见 tell）。
    tell(host, reply).map_err(|()| "operator:who")?;
    // 提示在**转授之后**：持树者据此可以按"提示一到，答话路必已在本表里"办事。
    // **这一对孔本函数不必拿着、也放不下**：本端那一枚（`link.rx()`）是垫的（本端从不读它），
    // 可它得**一直活着**——客人那一侧要有人认它（operator::client::open 的 `claim` 扫的就是
    push((*tip).ok_or("operator:tip")?, Tip::Guest(client)).map_err(|()| "operator:tell")
}

/// 认下持树者交回来的那一枚提示孔（**只认一次**）：判据两格——`owner == 持树者`
/// （那一枚是它铸的）**且** 记号 = TIP_MARK。认下来之后本线程拿着的就是
/// "往提示之路推客人号 / 协调帧 / 一条路"那一枚
pub fn host_of(
    host: TaskId,
    millis: Wait,
    tip: &mut Option<PieToken>,
) -> Result<TaskId, &'static str> {
    if tip.is_some() {
        return Ok(host);
    }
    // 交给调用方拿着：同一条路上以后每次都往里推客人号 / 协调帧 / 一条路（**同一枚线程**用）。
    *tip = establish::claim(host, TIP_MARK, millis);
    if tip.is_none() {
        return Err("operator:tip");
    }
    Ok(host)
}

/// 把**客人交出来的那一枚**转授给持树者
/// 转授的是"客人开的那扇门"（`owner` 是客人），持树者那侧认领时认的正是它
/// 子集只给 `R|W`，**不加 `VEST`**：持树者用这一枚写答话，不需要再授出——一分不多
pub(crate) fn hand(reply: PieToken, host: TaskId) -> Result<(), ()> {
    let hole = mail::HolePie::from_token(reply);
    port::ship(&hole, host, Access::FETCH | Access::STORE, Policy::NONE)
        .map(|_| ())
        .map_err(|_| ())
}

/// **一枚门牌落下去之后那三条读数**（land 每枚门牌返一行）
pub struct Landed {
    /// 落门牌那一步（`bind`）：答那一格自己的号
    pub land: Result<(), Fail>,
    /// 那一格自己的号（`land` 不成时是零号）
    pub plate: EntryId,
    /// 查回来验一遍（`token`）：**路译得回、那一枚门闩取得回来**
    pub find: Result<(), Fail>,
    /// 拿号问名：**号 ↔ 名对得上**，才算那枚号是真坐标
    pub named: Option<String>,
}

/// （`token`）→ 逐枚拿号问名（`name`）→ **每枚一行读数**
/// `tree` = 拿谁的会话；`family` = 哪一族（读数行前缀）；`road` = **绝对坐标的段列表**
/// （`["svc","principal"]`、`["svc","drv"]`、`["dev", 类]`）；`mine` = 那一格声不声明归属
/// 即返回次序**）
/// **`road` 是"容器链"，不含那一枚自己的名字**：`/svc/sys/principal` 那块窗格底下才放 `ask` / `set`
/// 故 `road = ["svc","principal"]` 而 `faces = [("ask",…),("set",…)]`；驱动那一家是两段
/// `["svc","drv"]`（砖就叫 `/svc/drv/router`）。把砖的名字也塞进 `road` 会**先立一块同名的
pub fn land(
    tree: &Face,
    family: &str,
    road: &Path,
    mine: Mine,
    permit: Permit,
    faces: &[(&str, PieToken)],
    millis: Wait,
) -> Vec<Landed> {
    let mut at: Option<EntryId> = None;
    for seg in road.iter() {
        let here = match at {
            Some(id) => Pane::of(tree, id),
            None => tree.root(),
        };
        match here.open(seg.to_string(), millis) {
            Ok(next) => at = Some(next.id()),
            Err(fail) => {
                debug::put(&alloc::format!(
                    "{family}: tree road={road} open at={seg:?} failed={fail:?}"
                ));
                return Vec::new();
            }
        }
    }
    let pane = match at {
        Some(id) => Pane::of(tree, id),
        None => tree.root(),
    };
    let root = tree.root();
    // 二·五、**先订一次**（序是契约）：这一处落格点自己那一块——`bind` 那一下树会把
    // `Landed`/`Rebound` 推到**订得起的人**的孔上。**订要持柄**：`watch` 是 `Grant::Watch`
    // 那一维上的一枚（`Face::rein` 借出来）；这一面拿不到那一柄权（或订不成）⇒ 退成
    // "每面再问一趟 `name`"，与从前逐字相同。
    // **只有"订得起"的那几处才订**：一枚订阅换掉的是**这一处每一面那一趟 `name`**——故
    // 单面那一档（本景里有十处：`router` / `rtc` 与八个设备类）是**纯亏**：省下的那一趟正好
    // 等于订出去的那一趟，而树上多挂一枚订阅、多一枚孔（实测：一律订的话那么一跑里
    // `watchers` 从 7 涨到 22）。面数 > 1 才订——那里一枚订阅换掉 n 趟。
    let rein = tree.rein(Grant::Watch);
    let mut watch = if faces.len() > 1 {
        rein.watch(road, millis).ok()
    } else {
        None
    };
    // 三、逐枚：落 → 查回来 → 拿号问名 → 一行读数。
    let mut out = Vec::with_capacity(faces.len());
    // **这一处走了哪几条路**（`via_event` = 那一对由树上推回来的事件说；`via_name` = 事件没收到、
    // 退回问了一次）：每个落格点一行、release 也看得见——**不然这一改有没有生效根本看不见**
    // （逐面那一行是 `debug!`，release 档是空的）。
    // `via_skip` = 这一处**按上面的判据没订**（单面那一档），故它走的就是从前的 `name` 那一趟。
    let (mut via_event, mut via_name, mut via_skip) = (0usize, 0usize, 0usize);
    for (face_name, entry) in faces {
        let face_name = *face_name;
        let name = face_name.to_string();
        // **落门牌**：答的是门牌自己那一格的号。
        let landed = pane
            .bind(name, *entry, permit, mine, millis)
            .map(|plate| plate.id());
        let (land, plate) = match &landed {
            Ok(id) => (Ok(()), *id),
            Err(fail) => (Err(*fail), EntryId::new(0)),
        };
        // `try_join` 答 `None` 只可能是"那一条路满了"（名字那一关上面已过）⇒ 折成
        // Fail::Full——与"装不下"是同一句话。**两面共用它**（查回来那一趟与"等事件"那一趟）。
        let full = road.try_join(face_name);
        let find = match &landed {
            Ok(_) => match &full {
                Some(full) => tree
                    .tile(full, millis)
                    .and_then(|tile| tile.token(millis))
                    .map(|_| ()),
                None => Err(Fail::Full),
            },
            Err(fail) => Err(*fail),
        };
        // **拿号问名**：号 ↔ 名这一对对得起来，才算那枚号是真坐标。
        // **这一条不再另问一趟**（乙）：`bind` 那一下树是**先把事件推到本端孔上、再答那一句**
        // （`serve/answer.rs` 的 `changed` 与 `serve/mod.rs` 的 `serve_one`——答话在 `answer`
        // 返回之后才发），故 `landed` 一回来，那一条已经排在本端这一枚孔上了；收下它即可
        // ——"那一格自己的号"与"从根写起的那条路"两格都在载荷里，正是本行要的那一对。
        // 收不到（本端孔的队列溢了 / 那一格被别条路顶掉 / 一开始就没订成）⇒ **退回问一次**：
        // 判据一字不差，只是那一趟又回来了（读数里 `via=` 那两格分得开）。
        let mut via = "event";
        let named = match &landed {
            Ok(id) => match &mut watch {
                Some(w) => match named_by_event(w, full.as_ref(), *id) {
                    Some(name) => Some(name),
                    None => {
                        via = "name";
                        root.name(*id, millis).ok()
                    }
                },
                None if faces.len() > 1 => {
                    // **订过、但没收到那一条**（队列溢了 / 那一格被别条路顶掉）⇒ 退回问一次。
                    via = "name";
                    root.name(*id, millis).ok()
                }
                None => {
                    // **本来就没订**（单面那一档）⇒ 与从前逐字相同。
                    via = "skip";
                    root.name(*id, millis).ok()
                }
            },
            Err(_) => None,
        };
        if landed.is_ok() {
            match via {
                "event" => via_event += 1,
                "skip" => via_skip += 1,
                _ => via_name += 1,
            }
        }
        // **落不成当场说一句**：读数不能走 `debug!`（release 档那是空）——本手是六个调用点
        // 共用的那一处，而其中 `principal` / `coalition` 两处**不成也照样起**，从前那两个域
        // 少落一格在 release 档里**没有出处**（树上看得出少一格，没人说得出为什么）。
        if land.is_err() || find.is_err() {
            debug::put(&alloc::format!(
                "{family}: tree land failed name={face_name} land={land:?} find={find:?} entry={} plate={} pname={} via={via}",
                entry.get(),
                plate.get(),
                named.as_ref().map(|name| name.as_str()).unwrap_or("-"),
            ));
        }
        debug!(
            "{family}: tree name={face_name} land={land:?} find={find:?} got={} entry={} plate={} pname={} via={via}",
            find.is_ok(),
            entry.get(),
            plate.get(),
            named.as_ref().map(|name| name.as_str()).unwrap_or("-"),
        );
        out.push(Landed {
            land,
            plate,
            find,
            named,
        });
    }
    // **一处一行**（每面那一行是 `debug!`，release 档是空的）：这一行是"每面少问一趟"那件事
    // 唯一的凭据——`via_event` 全中就是它成了；`via_name` 那一格一动，就是退回了问一次。
    debug::put(&alloc::format!(
        "{family}: tree land road={road} faces={} via_event={via_event} via_name={via_name} via_skip={via_skip} land_fail={} find_fail={}",
        faces.len(),
        out.iter().filter(|one| one.land.is_err()).count(),
        out.iter().filter(|one| one.find.is_err()).count(),
    ));
    out
}

/// **收那一条已经排到本端孔上的事件**（**非阻塞**）：路与号都对上 ⇒ 答那一段名。
///
/// # 为什么不必等
/// 树是**先推、后答**的：`answer` 里 `changed` 把事件推给订得起的人，而那一句答话在
/// `answer` 返回之后才发（`serve/mod.rs` 的 `serve_one`）。故 `bind` 的答话一到，这一条
/// 就已经排在本端那枚孔上了——"等"这一格因此没有期限可给，也不必给。
///
/// # 为什么要挑
/// 本端订的是**这一块**（`road`）：同族别的面的事件也会推到这枚孔上，故按**号 ＋ 路**
/// 两个判据挑；挑不中的丢掉继续。孔上排得下几只由内核说（`QUEUE_CAP`），故这一圈有界
/// ——转完仍没有 ⇒ 答 `None`，由调用方退回问一次 `name`（判据一字不差）。
fn named_by_event(watch: &mut Watch<'_>, full: Option<&PathBuf>, plate: EntryId) -> Option<String> {
    let full = full?;
    for _ in 0..8 {
        match watch.try_next() {
            Ok(Some(ev)) => {
                if ev.id == plate && ev.road.as_str() == full.as_str() {
                    return Some(full.file_name()?.to_string());
                }
            }
            // 孔上没有手 / 收不动 ⇒ 到此为止（由调用方退回问一次）。
            Ok(None) | Err(_) => return None,
        }
    }
    None
}
