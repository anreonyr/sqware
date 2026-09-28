//! operator::bridge — **上树那一趟在实现侧的两手**：装配者手里的持树者（那一枚号 ＋ 提示之路 ＋
//! 协调帧两格）请它做的两件事——**接一位客人上树**（[`Tree::attach`]）与**递一条路上去**
//! （[`Tree::plate`]）；以及**各域自己落门牌并自证**那一趟（[`land`]）。
//!
//! **同一趟的两侧**：[`Tree::plate`] 是**递上去**——本域不上树，立由持树者在自己核里做
//! （`programs/src/system/operator/plate.rs`）；[`land`] 是**自己落**——各域开一条树会话、逐段
//! 分路、按名字把门牌贴上去、再查回来验一遍。两侧共用一个坐标（`["sys","principal"]` 那种路）。
//!
//! **客人只有别的域**：装配者替每一位客人把孔转给持树者、再把它的号推上提示路。而"往树上立
//! 一格"**不由本域上树**——本域只把那一枚与那一条路递过去，**立由持树者在自己核里做**：树是
//! 那一格的权威，而它当不了自己的客人（自指 ⇒ 环）。
//!
//! **照实记（本文件从前只放装配侧）**：这一刀把四处逐字同构的"落门牌并自证"收成 [`land`]——
//! 名册 / 盟册两处服务、`driver::context::Context::plate`（三台驱动共用）、`uart::desk::plate`
//! （两枚门牌落在一块窗格里）；量出来的行数见 [`land`] 自己的照实记。故本文件从"只放装配侧"
//! 改成"放上树那一趟的两侧"。
//!
//! **照实记（后一刀：四处调用点只剩三个形状）**：名册与盟册原先各有一枚包着 `land` 的壳
//! （`serve_tree`：体就是一句 `land`，没有自己的状态与判断）——那一刀把它撤了，两处调用点直接
//! 叫 `land`。故今天 `land` 的调用点是**四处三形**：名册、盟册、三台驱动共用的一处、uart 一处。
//!
//! 三侧分家之后两侧共用的图与说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`protocol::system::operator`]。

use alloc::vec::Vec;

use env::Mark;
use env::Wait;
use env::wire::Eyes;
use env::wire::Field;
use env::{Name, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

use protocol::communication::establish;
use protocol::debug;
use protocol::system::operator::client::{Face, Mine, Pane};
use protocol::system::operator::{EntryId, Fail, Permit, Rule, TIP_LEN, Tip};
pub use protocol::system::operator::{LINK, TIP_MARK};

// ── 装配侧（装配者调用）──────────────────────────────────────

/// **推一句话过去**（提示之路那三形共用这一手：`Tip` 自己知道自己多长）。
///
/// **只走提示之路**：那条路上三形各带一格 `kind`（读者是持树者，它按首格认形状）。
fn push(into: PieToken, tip: Tip<'_>) -> Result<(), ()> {
    let mut rec = [0u8; TIP_LEN];
    let n = tip.store(&mut rec).ok_or(())?;
    mail::HolePie::from_token(into).push(&rec[..n]).map_err(|_| ())
}

/// **把一个号推过去**（`TaskId`，8 字节小端）——**树路上那一格**：告客人"答话的是谁"。
///
/// **这一条不走 [`Tip`]**（照实记：这一刀在这里栽过一次）：读它的是
/// [`communication::session`](protocol::communication::session) 的 `hear`——那条路由**板**与**树**
/// 两族共用，它认的是**恰好 8 字节的裸号**。给这一帧套上提示之路那种 `kind` 头，`hear` 当场读
/// 不成（`n == 8` 不成立）⇒ `Fail::Link` ⇒ 名册一上树就起不来（实测：`exit tid=5 reason=0xa`）。
/// 故**两条路两种帧形**：提示之路带 `kind`（三形一张表），这条路只有一枚号。
///
/// **帧形只有一处**：宽度与字节序归 [`Field`] 给 [`TaskId`] 那一对 `store` / `fetch`。
pub(crate) fn tell(who: TaskId, into: PieToken) -> Result<(), ()> {
    let mut rec = [0u8; TaskId::WIDTH];
    who.store(&mut rec);
    mail::HolePie::from_token(into).push(&rec).map_err(|_| ())
}

/// **协调那一帧要带的两格**：名册那一位域的号 / 盟册那一位域的号（`None` = 还没到）。
///
/// 一格一个位、**可以分两帧到**（次序不定），故装配者收着、持树者补着，都不当"一次性解出来"。
/// 两侧共读这一个类型（`server.rs` 原先自己写了一份同形的私有 `Coord`）。
#[derive(Clone, Copy, Default)]
pub struct Coord {
    pub roster: Option<TaskId>,
    pub league: Option<TaskId>,
}

impl Coord {
    /// 递帧要的两对：**槽位由眼睛的名字定**，不由位置定。
    fn pairs(self) -> [(Option<TaskId>, Eyes); 2] {
        [(self.roster, Eyes::Roster), (self.league, Eyes::League)]
    }
}

/// **持树者在装配者这一侧的状态**：持树者的号 ＋ 它那条提示之路 ＋ 协调帧那两格。
///
/// 它们问的是**树的语义**——客人怎么接、提示怎么认、哪一双眼睛往哪记——故收进树这一间。
#[derive(Default)]
pub struct Tree {
    host: Option<TaskId>,
    tip: Option<PieToken>,
    coord: Coord,
}

impl Tree {
    /// 持树者那一枚的号（`None` = 还没起）。
    pub fn host(&self) -> Option<TaskId> {
        self.host
    }

    /// **把这位客人接上树**（三步见 [`attach`]）。持树者还没起就没得接。
    pub fn attach(&mut self, client: TaskId, millis: Wait) -> Result<(), &'static str> {
        let host = self.host.ok_or("no tree yet")?;
        attach(client, host, millis, &mut self.tip, self.coord)
    }

    /// **它就是持树者本身**：认下它那条提示之路，此后客人上树才有路可走。
    pub fn adopt(&mut self, host: TaskId, millis: Wait) -> Result<(), &'static str> {
        self.host = Some(host);
        self.tip = None;
        host_of(host, millis, &mut self.tip)?;
        Ok(())
    }

    /// **递一条路上去**：请持树者把这条路上的窗格逐段立出来（缺的就地造），末段按 `leaf`
    /// 落叶子，或立窗格（`leaf = None` ⇒ 末段是**窗格**）。
    ///
    /// 路是**绝对坐标**（从根起数）：`["sys","control"]`、`["sys","operator"]`、
    /// `["sys","operator","part"]` 三种落法**同一个形状**——连"父底下立一块窗格"、再深一层
    /// 也说得出来（从前那一版"两段名字 ＋ 一格 layer"说不出第四种）。
    ///
    /// **次序是契约**：**先交那一枚、再推帧**——持树者一见帧就要立，而立要那一枚已经在它表里
    /// （帧里带的是它在**树表里**的号）。
    ///
    /// **本域不上树**：立由持树者在自己核里做（`operator::plate::plate`），本域只是递东西的那
    /// 一侧 ⇒ 这里不要会话、也不要名册上的身份（`land` 那道门是给**客人**的，本域不是客人）。
    ///
    /// `host` / `tip` 有一格没在（这一景没有持树者）⇒ `Err`：那是景的事，不是本手的失败。
    /// `rule` = **这一趟要不要在这一格上带一句规矩**。装配者**报不出任何号**（没有名录面、
    /// 也没有读格的手）⇒ 它在这条路上说得出的只有 [`Rule::Root`] 那一句（"许给根"），
    /// 而"带哪一条"由**持树者**落格时写进那一格（见 `programs/src/system/operator/plate.rs`）。
    pub fn plate(
        &mut self,
        road: &[Name],
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
        push(tip, Tip::Plate { road, leaf, rule }).map_err(|()| "operator:plate")
    }

    /// **它是哪一双眼睛**：把那一格记进给持树者的协调帧（重复推是幂等的）。
    pub fn eye(&mut self, eyes: Eyes, who: TaskId) {
        match eyes {
            Eyes::Roster => self.coord.roster = Some(who),
            Eyes::League => self.coord.league = Some(who),
        }
    }
}

/// 把持树者接上一位客人（装配者调用）：**三步**。
///
/// `host` = 持树者的号（`service::spawn` 交回来的那个，装配者本来就知道它）。
/// `tip` = 提示之路在**本线程表里**的那一枚（第一次用时认下来，此后逐条传下去）。
/// `coord` = 协调那一帧要带的两格（**哪一位域** + **它是哪一双眼睛**）：那位递门牌的域那一格
/// 才有值，其余留空——**定长两格**，因为这一族只有两双眼睛（[`Eyes`]）。
///
/// 返 `Err(哪一步)`：名字非法 / 席位满 / 等不到客人那一枚 / 提示孔认不到……对调用方是
/// 同一件事——**这条服务没接上树**——但"死在哪一步"正是装配诊断要的那一格。
pub fn attach(
    client: TaskId,
    host: TaskId,
    millis: Wait,
    tip: &mut Option<PieToken>,
    coord: Coord,
) -> Result<(), &'static str> {
    let link = Name::new(LINK).map_err(|_| "operator:name")?;
    // 1+2. **一手就是"两头都装"**：本端那一枚交出去（落在本域表里——客人拿不到它，也不需要：
    //      答话从客人自己那枚走）＋ 认领**这位客人**交出来的那一枚（记号 = 这条路的名字）。
    //      判据两格（`owner == client` ＋ 记号）与原 `seat` ＋ `claim` 逐字同源。
    let link =
        establish::endpoint(client, Mark::of(link.as_str()), millis).map_err(|_| "operator:seat")?;
    // **认不到对端那一枚 = 这条路没接上**（原 `claim` 那一格）。
    if link.tx().is_none() {
        return Err("operator:claim");
    }
    // 3. 提示孔（只认一次）→ 把客人那一枚转授给持树者 → 告两边。
    //    `host_of` 之后 `tip` 必有值（认不到它自己就返 `Err` 了）——所以这里取的是**那一枚孔**，
    //    要推的正是它（**不是** `host`：那是持树者的号，推不动）。
    let _ = host_of(host, millis, tip)?;
    let tip_at = (*tip).ok_or("operator:tip")?;
    // 3.5 **协调那一帧**：把递门牌那几位域的号推过去。**门牌不由这里转授**（那是各域自己
    // 在落完门牌之后直接交给持树者的）。
    // 次序仍是契约：客人号来之前，持树者先认出名册那一枚门牌（它按 `owner` + 记号找）；
    // 两帧按位递、次序不定，收到哪一枚就补上哪一枚（对齐见 `server.rs` 的 `settle`）。
    for (who, eyes) in coord.pairs() {
        if let Some(who) = who {
            push(tip_at, Tip::Coord { who, eyes }).map_err(|()| "operator:coord")?;
        }
    }
    // 树路上本端手里那一枚 = **客人答话路的写端**（认下来时进 `link.tx()`）。
    let reply = link.tx().ok_or("operator:hand")?;
    hand(reply, host).map_err(|()| "operator:hand")?;
    // 客人那一侧的一格：**答话的是谁**（持树者的号，8 字节**裸号**——那条路的读者是
    // `communication::session::hear`，见 [`tell`]）。
    tell(host, reply).map_err(|()| "operator:who")?;
    // 提示在**转授之后**：持树者据此可以按"提示一到，答话路必已在本表里"办事。
    // **这一对孔本函数不必拿着、也放不下**：本端那一枚（`link.rx()`）是垫的（本端从不读它），
    // 可它得**一直活着**——客人那一侧要有人认它（`operator::client::open` 的 `claim` 扫的就是
    // 本域铸出去那一枚的副本），而认下之后持树者那一路也一直指着它写。它归**本域那张表**
    // （`Endpoint` 上只有 `claim`，没有"放下"这个动作）⇒ 本域退场时一并回收。
    push((*tip).ok_or("operator:tip")?, Tip::Guest(client)).map_err(|()| "operator:tell")
}

/// 认下持树者交回来的那一枚提示孔（**只认一次**）：判据两格——`owner == 持树者`
/// （那一枚是它铸的）**且** 记号 = [`TIP_MARK`]。认下来之后本线程拿着的就是
/// "往提示之路推客人号 / 协调帧 / 一条路"那一枚。
///
/// **只认、不铸**：本端这一侧在这条路上不需要自己那一枚。**照实记（这一格原先还多装了一条）**：
/// 从前这里先 `seat` 一次——本端另铸一枚、刻的是另一个记号（`TIP_NAME = "operator-tip"`）——
/// 而那一枚两头都不用。两个记号并成一个之后，这条路上只剩这一手。
///
/// **两种装配者都用它**：编排域用它把客人接上树（[`attach`] 的第一步），引导域用它
/// 把这条提示之路先认到手里、再转授给编排域（`root` 引导期的交接那一格）。
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
///
/// 转授的是"客人开的那扇门"（`owner` 是客人），持树者那侧认领时认的正是它。
///
/// 子集只给 `R|W`，**不加 `VEST`**：持树者用这一枚写答话，不需要再授出——一分不多。
pub(crate) fn hand(reply: PieToken, host: TaskId) -> Result<(), ()> {
    let hole = mail::HolePie::from_token(reply);
    port::ship(&hole, host, Access::FETCH | Access::STORE, Policy::NONE)
        .map(|_| ())
        .map_err(|_| ())
}

// ── 上树那一趟（客人侧：各域自己落门牌并自证）─────────────────

/// **一枚门牌落下去之后那三条读数**（[`land`] 每枚门牌返一行）。
///
/// **三格就是那一趟的三步**：落（`bind`）、查回来（`token`，旧 `find`）、拿号问名（`name`）。
/// 读数行里另两格是这两个 `Result` 的第二说法，故**不另存**：`plate` 就是 `land` 答出来的那枚号，
/// `got` 就是 `find` 成没成——**一格写两遍是 §3 那个死格**（原来的四处各写了一遍）。
pub struct Landed {
    /// 落门牌那一步（`bind`）：答那一格自己的号。
    pub land: Result<(), Fail>,
    /// 那一格自己的号（`land` 不成时是零号）。
    pub plate: EntryId,
    /// 查回来验一遍（`token`）：**路译得回、那一枚门闩取得回来**。
    pub find: Result<(), Fail>,
    /// 拿号问名：**号 ↔ 名对得上**，才算那枚号是真坐标。
    pub named: Option<Name>,
}

/// **上树落门牌那一趟**：名字先全验 → 逐段分路（`part` 幂等）→ 逐枚落（`bind`）→ 逐枚查回来
/// （`token`）→ 逐枚拿号问名（`name`）→ **每枚一行读数**。
///
/// `tree` = 拿谁的会话；`family` = 哪一族（读数行前缀）；`road` = **绝对坐标的段列表**
/// （`["svc","principal"]`、`["svc","drv"]`、`["dev", 类]`）；`mine` = 那一格声不声明归属；
/// `permit` = **这一趟落的每一格带哪一句许可**；`faces` = 要落的那几枚（**末段名 ＋ 入口，次序
/// 即返回次序**）。
///
/// **`road` 是"容器链"，不含那一枚自己的名字**：`/svc/principal` 那块窗格底下才放 `ask` / `set`，
/// 故 `road = ["svc","principal"]` 而 `faces = [("ask",…),("set",…)]`；驱动那一家是两段
/// `["svc","drv"]`（砖就叫 `/svc/drv/router`）。把砖的名字也塞进 `road` 会**先立一块同名的
/// 窗格、再把砖落在它底下**——这一格栽过，照实记在 `Context::plate` 那一处（一处路的写法，五台程序
/// 的读数一起变）。
///
/// # `permit` 那一格（照实记：它从前**没有**形参）
///
/// 前四处的调用点（名册 / 盟册 / 驱动 / `uart`）今天全传 [`Permit::Unset`]，而**设备账那一台
/// 传的是 `Permit::Among(c_类)`**——它落的每一格 `/dev/<类>/<名>` 都只许**那一类的盟**里的人找
/// （"许不许你驱这一类"那条规矩的落点）。故这一格从"写死"变成形参：**它有一个真的选择者了**，
/// 而选择者只有一位（hub）、且**按类分趟落**（一趟一个 permit ⇒ 一个形参够）。
///
/// **返落到的那几枚**：名字非法、或路在某一段断了 ⇒ **空表**（那时读数已印一行）。故"路这一趟
/// 断没断"与"某一枚落没落上"分得开，不由一个 `Result` 折成同一格。
///
/// # 照实记（这一趟是量出来的：四处逐字同构）
///
/// ```text
///   名册那处 serve_tree（**后一刀已撤壳**）  50 行 / 45 码行 / 2 枚   tile+token → name
///   盟册那处 serve_tree（**后一刀已撤壳**）  50 行 / 45 码行 / 2 枚   同上
///   Context::plate（三台驱动共用）        59 行 / 52 码行 / 1 枚   同上（含末尾六条断言）
///   uart::desk::plate                    66 行 / 63 码行 / 2 枚   只有 name（两枚落在一块窗格里）
/// ```
///
/// 前两处的 50 行里有 **44 行逐字相同**——差的 6 行：4 行是 `debug!` 里那个族名前缀、1 行多一句
/// 注释、1 行是签名（一家由 `Session` 现取 `Face`、一家收 `&TreeFace`）。四处合计 225 行 /
/// 204 码行，步骤一字不差；故"两台以上逐字同构 ⇒ 收"在这里成立。**那 44 行相同里还有一层**：
/// 两处服务那一对连"把 `[(Grant, PieToken, Name); 2]` 摊成 `[(str, PieToken)]`"都逐字同构——
/// 那一对壳撤掉之后，摊开那两行并进了各处的调用点，`land` 的签名一个字没动。
///
/// **照实记（uart 那一处换了自证口径：多两趟往返）**：uart 原来只问一句 `name`（"一次问完两格"：
/// 既答名、也证明那一号还在），收进来之后**四家同一条口径**——`token`（路译得回 ＋ 那一枚门闩
/// 取得回来）**再加** `name`（号 ↔ 名）。故 uart 那两枚门牌各多两问（`tile` ＋ `token`）：boot 期
/// 四问，不在稳态。这是本刀唯一一处**拿往返换口径**，记在这（**一处口径**比**两趟往返**值）。
///
/// **照实记（许可那一格从"写死"变形的理由）**：四处的调用点从前全写 `Permit::Unset`，故那时
/// "设一个形参就是 §3 那个**报法字段**（各点各硬编码一个值 ⇒ 它不是数据，是把若干特例塞进签名）"。
/// 设备账那一台是**第一位真的选择者**（它按类写 `Among(c_类)`），故那一格今天跟 `TipIn::Plate`
/// 那一格一样，是**数据**了。
///
/// **照实记（收源码不等于减镜像：一笔要分开记的逆数）**：四处 204 码行 ⇒ 本手 83 行 ＋ 四个薄
/// 调用点，同构从 4 份收到 1 份；可**每台摸得到它的程序各静态链一份**，量出来是：
///
/// ```text
///   .text（llvm-size，去掉页对齐）  rtc +3484   router +3118   coalition +3290   uart +1374 B
///   剥符号后的 ELF **按页对齐**     ⇒ 每台顶出一整页 / 两页
///   initrd                        1238469 → 1271237 B（+32768，+2.65%）
/// ```
///
/// 故这一刀的账分开记：**同构 −3 份、口径一处**（收成）；**镜像 +32 KB**（付的）。真实码增约
/// 11 KB，页对齐把它放大成 32 KB 的台阶——"收"买的是同构只留一份，不是省下镜像。
///
/// **走的是号、不是柄**：`Pane::open` 借出来的下一块窗格活不过这一轮（自指的借用），故这一趟只把
/// "到哪儿了"记成一枚 [`EntryId`]，每段就地从 `tree` 造一块柄。
pub fn land(
    tree: &Face,
    family: &str,
    road: &[&str],
    mine: Mine,
    permit: Permit,
    faces: &[(&str, PieToken)],
    millis: Wait,
) -> Vec<Landed> {
    // 一、名字**先全验**：一个不合法就一趟都不上路（与"路上哪一段没分出来"分开报）。
    let mut names: Vec<Name> = Vec::with_capacity(road.len());
    for seg in road {
        match Name::new(seg) {
            Ok(name) => names.push(name),
            Err(_) => {
                debug!("{family}: tree: bad name");
                return Vec::new();
            }
        }
    }
    // 二、逐段分路（`open` 幂等：那一格已经在就答它那个号；"是不是窗格"由 `part` 自己判）。
    let mut at: Option<EntryId> = None;
    for (seg, name) in road.iter().zip(&names) {
        let here = match at {
            Some(id) => Pane::of(tree, id),
            None => tree.root(),
        };
        match here.open(*name, millis) {
            Ok(next) => at = Some(next.id()),
            Err(fail) => {
                debug!("{family}: tree road={road:?} open at={seg} failed={fail:?}");
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
        let Ok(name) = Name::new(face_name) else {
            debug!("{family}: tree: bad name");
            return out;
        };
        // **落门牌**：答的是门牌自己那一格的号。
        let landed = pane
            .bind(name, *entry, permit, mine, millis)
            .map(|plate| plate.id());
        let (land, plate) = match &landed {
            Ok(id) => (Ok(()), *id),
            Err(fail) => (Err(*fail), EntryId::new(0)),
        };
        // **查回来验一遍**：按路（这一段名字只在这里再用一次，此后一律按号）。
        let find = match &landed {
            Ok(_) => {
                names.push(name);
                let found = tree
                    .tile(&names, millis)
                    .and_then(|tile| tile.token(millis))
                    .map(|_| ());
                names.pop();
                found
            }
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
