//! operator::bridge — **上树那一趟在实现侧的两手**：装配者手里的持树者（那一枚号 ＋ 提示之路 ＋
//! 协调帧两格）请它做的两件事——**接一位客人上树**（[`Tree::attach`]）与**递一条路上去**
//! （[`Tree::plate`]）；以及**各域自己落门牌并自证**那一趟（[`land`]）。
//! **同一趟的两侧**：[`Tree::plate`] 是**递上去**——本域不上树，立由持树者在自己核里做
//! （`programs/src/system/operator/plate.rs`）；[`land`] 是**自己落**——各域开一条树会话、逐段
//! 分路、按名字把门牌贴上去、再查回来验一遍。两侧共用一个坐标（`["sys","principal"]` 那种路）。
//! **客人只有别的域**：装配者替每一位客人把孔转给持树者、再把它的号推上提示路。而"往树上立
//! 一格"**不由本域上树**——本域只把那一枚与那一条路递过去，**立由持树者在自己核里做**：树是
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
use crate::system::control::{BOOT_MS,  Service};
use crate::unit::UnitFile;

use protocol::communication::establish;
use protocol::debug;
use protocol::service::operator::client::{Face, Mine, Pane};
use protocol::common::path::Path;
use protocol::service::operator::{EntryId, Fail, Permit, Rule, TIP_LEN, Tip};
pub use protocol::service::operator::{LINK, TIP_MARK};

/// **推一句话过去**（提示之路那三形共用这一手：`Tip` 自己知道自己多长）。
/// **只走提示之路**：那条路上三形各带一格 `kind`（读者是持树者，它按首格认形状）。
fn push(into: PieToken, tip: Tip) -> Result<(), ()> {
    let mut rec = [0u8; TIP_LEN];
    let n = tip.store(&mut rec).ok_or(())?;
    let road = mail::HolePie::from_token(into);
    road.push(&rec[..n], Wait::Forever).map_err(|_| ())?;
    road.wait(HoleDir::Push, Wait::Forever).map_err(|_| ())?;
    Ok(())
}

/// **把一个号推过去**（`TaskId`，8 字节小端）——**树路上那一格**：告客人"答话的是谁"。
pub(crate) fn tell(who: TaskId, into: PieToken) -> Result<(), ()> {
    let mut rec = [0u8; TaskId::WIDTH];
    who.store(&mut rec);
    let road = mail::HolePie::from_token(into);
    road.push(&rec, Wait::Forever).map_err(|_| ())?;
    road.wait(HoleDir::Push, Wait::Forever).map_err(|_| ())?;
    Ok(())
}

/// **持树者在装配者这一侧的状态**：持树者的号 ＋ 它那条提示之路。
/// 它们问的是**树的语义**——客人怎么接、提示怎么认——故收进树这一间。
#[derive(Default)]
pub struct Tree {
    host: Option<TaskId>,
    tip: Option<PieToken>,
}

impl Tree {
    /// 持树者那一枚的号（`None` = 还没起）。
    pub fn host(&self) -> Option<TaskId> {
        self.host
    }

    /// **把这位客人接上树**（三步见 [`attach`]）。持树者还没起就没得接。
    pub fn attach(&mut self, client: TaskId, millis: Wait) -> Result<(), &'static str> {
        let host = self.host.ok_or("no tree yet")?;
        attach(client, host, millis, &mut self.tip)
    }

    /// **门禁接线**：告诉持树者"名册那一面已经认下来了"——从那以后它那道门问得动身份。
    pub fn wire(&mut self) -> Result<(), &'static str> {
        let Some(tip) = self.tip else {
            return Err("no tip");
        };
        push(tip, Tip::Wired).map_err(|()| "operator:wire")
    }

    /// **它就是持树者本身**：认下它那条提示之路，此后客人上树才有路可走。
    pub fn adopt(&mut self, host: TaskId, millis: Wait) -> Result<(), &'static str> {
        self.host = Some(host);
        self.tip = None;
        host_of(host, millis, &mut self.tip)?;
        Ok(())
    }

    /// **这一台是不是持树者**——判据是**它自己交出来的那一件东西**：提示之路上那枚挂在它名下的
    /// `TIP_MARK` 孔（`establish::find` **只看**，不另铸一枚新的）。
    pub fn holds(&self, host: TaskId) -> bool {
        establish::find(host, TIP_MARK).is_some()
    }

    /// **递一条路上去**：请持树者把这条路上的窗格逐段立出来（缺的就地造），末段按 `leaf`
    /// 落叶子，或立窗格（`leaf = None` ⇒ 末段是**窗格**）。
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

/// 把持树者接上一位客人（装配者调用）：**三步**。
/// `host` = 持树者的号（`service::spawn` 交回来的那个，装配者本来就知道它）。
/// `tip` = 提示之路在**本线程表里**的那一枚（第一次用时认下来，此后逐条传下去）。
/// 返 `Err(哪一步)`：名字非法 / 席位满 / 等不到客人那一枚 / 提示孔认不到……对调用方是
/// 同一件事——**这条服务没接上树**——但"死在哪一步"正是装配诊断要的那一格。
pub fn attach(
    client: TaskId,
    host: TaskId,
    millis: Wait,
    tip: &mut Option<PieToken>,
) -> Result<(), &'static str> {
    // 1+2. **一手就是"两头都装"**：本端那一枚交出去（落在本域表里——客人拿不到它，也不需要：
    //      答话从客人自己那枚走）＋ 认领**这位客人**交出来的那一枚（记号 = 这条路的名字）。
    //      判据两格（`owner == client` ＋ 记号）与原 `seat` ＋ `claim` 逐字同源。
    let link = establish::endpoint(client, Mark::of(LINK), millis).map_err(|_| "operator:seat")?;
    // **认不到对端那一枚 = 这条路没接上**。
    if link.tx().is_none() {
        return Err("operator:claim");
    }
    // 3. 提示孔（只认一次）→ 把客人那一枚转授给持树者 → 告两边。
    //    `host_of` 之后 `tip` 必有值（认不到它自己就返 `Err` 了）——所以这里取的是**那一枚孔**，
    //    要推的正是它（**不是** `host`：那是持树者的号，推不动）。
    let _ = host_of(host, millis, tip)?;
    let reply = link.tx().ok_or("operator:hand")?;
    hand(reply, host).map_err(|()| "operator:hand")?;
    // 客人那一侧的一格：**答话的是谁**（持树者的号，8 字节**裸号**——那条路的读者是
    // `communication::session::hear`，见 [`tell`]）。
    tell(host, reply).map_err(|()| "operator:who")?;
    // 提示在**转授之后**：持树者据此可以按"提示一到，答话路必已在本表里"办事。
    // **这一对孔本函数不必拿着、也放不下**：本端那一枚（`link.rx()`）是垫的（本端从不读它），
    // 可它得**一直活着**——客人那一侧要有人认它（`operator::client::open` 的 `claim` 扫的就是
    // 本域铸出去那一枚的副本），而认下之后持树者那一路也一直指着它写。它归**本域那张表**
    push((*tip).ok_or("operator:tip")?, Tip::Guest(client)).map_err(|()| "operator:tell")
}

/// 认下持树者交回来的那一枚提示孔（**只认一次**）：判据两格——`owner == 持树者`
/// （那一枚是它铸的）**且** 记号 = [`TIP_MARK`]。认下来之后本线程拿着的就是
/// "往提示之路推客人号 / 协调帧 / 一条路"那一枚。
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

/// 把**客人交出来的那一枚**转授给持树者。
/// 转授的是"客人开的那扇门"（`owner` 是客人），持树者那侧认领时认的正是它。
/// 子集只给 `R|W`，**不加 `VEST`**：持树者用这一枚写答话，不需要再授出——一分不多。
pub(crate) fn hand(reply: PieToken, host: TaskId) -> Result<(), ()> {
    let hole = mail::HolePie::from_token(reply);
    port::ship(&hole, host, Access::FETCH | Access::STORE, Policy::NONE)
        .map(|_| ())
        .map_err(|_| ())
}

/// **一枚门牌落下去之后那三条读数**（[`land`] 每枚门牌返一行）。
pub struct Landed {
    /// 落门牌那一步（`bind`）：答那一格自己的号。
    pub land: Result<(), Fail>,
    /// 那一格自己的号（`land` 不成时是零号）。
    pub plate: EntryId,
    /// 查回来验一遍（`token`）：**路译得回、那一枚门闩取得回来**。
    pub find: Result<(), Fail>,
    /// 拿号问名：**号 ↔ 名对得上**，才算那枚号是真坐标。
    pub named: Option<String>,
}

/// **上树落门牌那一趟**：名字先全验 → 逐段分路（`part` 幂等）→ 逐枚落（`bind`）→ 逐枚查回来
/// （`token`）→ 逐枚拿号问名（`name`）→ **每枚一行读数**。
/// `tree` = 拿谁的会话；`family` = 哪一族（读数行前缀）；`road` = **绝对坐标的段列表**
/// （`["svc","principal"]`、`["svc","drv"]`、`["dev", 类]`）；`mine` = 那一格声不声明归属；
/// `permit` = **这一趟落的每一格带哪一句许可**；`faces` = 要落的那几枚（**末段名 ＋ 入口，次序
/// 即返回次序**）。
/// **`road` 是"容器链"，不含那一枚自己的名字**：`/svc/sys/principal` 那块窗格底下才放 `ask` / `set`，
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
    // 三、逐枚：落 → 查回来 → 拿号问名 → 一行读数。
    let mut out = Vec::with_capacity(faces.len());
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
        // **查回来验一遍**：按路（那一条路在这里再拼一次，此后一律按号）。
        // `try_join` 答 `None` 只可能是"那一条路满了"（名字那一关上面已过）⇒ 折成
        // [`Fail::Full`]——与"装不下"是同一句话。
        let find = match &landed {
            Ok(_) => match road.try_join(face_name) {
                Some(full) => tree
                    .tile(&full, millis)
                    .and_then(|tile| tile.token(millis))
                    .map(|_| ()),
                None => Err(Fail::Full),
            },
            Err(fail) => Err(*fail),
        };
        // **拿号问名**：号 ↔ 名这一对对得起来，才算那枚号是真坐标。
        let named = match &landed {
            Ok(id) => root.name(*id, millis).ok(),
            Err(_) => None,
        };
        debug!(
            "{family}: tree name={face_name} land={land:?} find={find:?} got={} entry={} plate={} pname={}",
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
    out
}
