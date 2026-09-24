//! service — **装配的机器**：把一张装配单变成"一批起好的服务"。
//!
//! 这张机器只认单子（`Program`）与清单（`Catalog`），不认具体是哪些服务——**单子住在
//! 各自的域里**：引导域那张只有一条（编排者），编排域那张是它自己要起的那几条（持树者排第一）。
//! 于是"引导域不知道系统里还有什么服务"这条不是靠自律，是靠**它手里没有那张单**。
//!
//! ```text
//!   装配单（各域私有）  Program { name, announce, tokens, channels, needs, board, operator, bind, holds_tree, died }
//!   清单（两种来源）    Catalog  ── 引导域：boot 借映那块；编排域：它从固件领来的只读视图
//! ```
//!
//! 想加第三条服务：在**编排域那张单**里加一行，本文件一个字都不用改。
//!
//! # 配给从哪来
//!
//! 装配者自己**不持**设备门闩——它在引导域手里。故发货走一次往返：
//! [`protocol::driver::supply::client::draw`] 把"要哪几样"递过去，固件把门闩直接授进**客人**的表里并回一段记录，
//! 装配者再把这**一段字节原样**投到客人那条通道上（客人按记号认领、**按位次归位**——位置即格）。
//! 装配者经手的只有字节，**一枚原件都不经过它**。
//!
//! # 身份从哪来
//!
//! 装配期每一条服务，都由装配者向**身份服务**要一条号、把它绑到那一条服务的代表线程上
//! （`derive(root)` + `bind(rep, p)`）——**在 `Hatch` 放行之前**，故服务一起来
//! `resolve(self)` 就答得出。身份服务自己是 `plan` 里 [`Eyes::Roster`] 那一条：它放行之后，
//! 本域先认下它交给生我者的门牌，再把**它自己与树**补绑上（那两条起来时它还没在）。
//! **装配者自己不绑**——它是写名册的那一个，不是被写的那一个。

use core::time::Duration;
use env::Mark;

use crate::supervisor::system::server::{self as service, Grant};
use env::wire::manifest;
use env::{Name, PieToken, TaskId};
use protocol::principal::client::Face;
use protocol::principal::core::PrincipalId;
use protocol::session::call as scall;
use protocol::session::{Pier, Quay};
use protocol::system::board::call as bcall;
use protocol::system::desk::{Announce, Table};
use runtime::env::room;

use crate::supervisor::operator::bridge as operator;
use crate::supervisor::system::board::bridge as board;

use protocol::driver::supply;
use protocol::driver::supply::call::{Need, WANT_MAX, Want};

use crate::supervisor::root::boot;
use crate::supervisor::system::machine::Machine;

/// 装配失败的编号——定义见 [`env::assembly::Died`]（本处只是转发）。
pub use env::assembly::Died;
/// **哪一双眼睛**也是装配单上的一格，故两处共读同一个定义。
pub use env::assembly::Eyes;

/// 认身份门牌 / 与它说话的短等间隔（毫秒）：门牌由 Server 起手交出，这里只是短等。
const RETRY_MS: usize = 1;

/// 一条服务的装配契约。
pub struct Program {
    /// 清单里的程序名 —— 也是本域给它起的服务名（`Build` 的名字与清单名同源）。
    pub name: &'static str,
    /// 怎么算"起来了"。
    pub announce: Announce,
    /// 起跑前要交到它手里的门闩（今天都是空的：门闩在起来之后才配）。
    pub tokens: &'static [Grant],
    /// 与它之间要开的通道（双方按名字对位，不靠位置约定）。
    pub channels: &'static [&'static str],
    /// 它要的门闩（`None` = 什么都不要，如调试回显）。
    ///
    /// **就是单子上的那几条**（[`Need`]）：那几张表由**收方**自己开
    /// （[`crate::driver::router::needs`]、[`crate::driver::uart::needs`] 与
    /// [`env::assembly::LODGER_WANTS`]），本域照单递出去、并在递之前把"类"翻成"哪一段区"
    /// （见 [`wire`]）——中间不再有"需求 → 单子"的转换。
    pub needs: Option<&'static [Need]>,
    /// 要不要板那条路（[`board::attach`]）。
    ///
    /// **它不是 `channels` 里的一行**：那几条是"放行前先装好、起来时交回"，而板那条路由
    /// **客人在起来之后自己装**（客人才是"问"的那一侧），故本域是等它，不是先装。
    /// 两端共用这一格：`true` ⇒ 它一定调 [`board::open`]（不然本域要白等一期）。
    pub board: bool,
    /// 要不要树那条路（[`operator::attach`]）。
    ///
    /// 与 [`Program::board`] 同一个形状、同一格位置（"两端共用"）：`true` ⇒ 它一定调
    /// [`operator::open`]。**按需发**——拿到这条路的服务，就能动整棵树（owner 归 Principal，
    /// 准入由门外那一问判，见 `protocol::operator::{judge, gate}`）。
    pub operator: bool,
    /// **装配期给不给它一条身份**（`derive(ROOT)` + `bind`）。
    ///
    /// 默认 `true`（每一条都绑，与从前一致）。**`false` 是给负证客人的**：门禁那条判据
    /// 里"没绑身份 ⇒ 拒绝"（`operator::judge` 的第一格）今天在真机上**没有反例**——11 台
    /// 客人全都是已绑身份、全放行。要读出"拒得住"，就得有一位**真的没身份**的客人去撞门。
    ///
    /// 与 [`Program::board`] / [`Program::operator`] 同一形状（两端共用）：`false` ⇒ 装配者
    /// **不**给它绑，它自己 `resolve(self)` 会答 `None`。
    pub bind: bool,
    /// **它就是持树者本身**（不是树的客人）：起来之后本域把它那条提示之路认到手，此后每位
    /// 上树的客人都往那条路上递号（见 [`assemble`] 的第二段）。
    ///
    /// **它必须排在 `plan` 第一位**：排在它前面的客人没树可上——那一支会报 `no tree yet`
    /// （此刻本域手里还没有持树者的号）。今天只有编排域那张单上有这一条。
    pub holds_tree: bool,
    /// **它是持树者的哪一双眼睛**（见 [`Eyes`]）——`None` = 不是（绝大多数行都不是）。
    ///
    /// **照实记（这一格是从字符串比对改过来的）**：原先装配机器拿**名字**认这两条
    /// （`p.name == "principal"` / `== "coalition"`）——装配单上把那一行改个名，这一段就
    /// **静默失灵**（门禁从此判不了身份，机器要到真机上才红）。现在与 [`Program::holds_tree`]
    /// 同一形状：写在单子上、机器照格办事。
    pub eyes: Option<Eyes>,
    /// 装配死在这一条时报哪个号。
    pub died: Died,
}

/// 清单的读面：装配者按名字挑镜像。
///
/// 两种来源**同一形状**：引导域手里是 boot 借映的那块字节，编排域手里是它从固件领来的
/// 那段只读视图（同一批物理页、各自的 VA）。清单里的镜像是**相对 blob 的切片**，故换一张
/// 表、换一个 VA 都照样解析得出来——这正是"零拷贝把这片区交出去"能成立的原因。
pub struct Catalog<'a> {
    view: &'a [u8],
}

impl<'a> Catalog<'a> {
    /// 拿一块字节当清单。`None` = 清单头非法（条数为零 / 超上限 / 装不下）。
    pub fn new(view: &'a [u8]) -> Option<Catalog<'a>> {
        manifest::Entries::new(view)?;
        Some(Catalog { view })
    }

    /// boot 借映给引导域的那块。
    pub fn of_boot(boot: &boot::Root) -> Option<Catalog<'static>> {
        Catalog::new(boot.view())
    }

    /// 从清单里挑出这个程序。
    pub fn find(&self, want: &str) -> Option<manifest::Entry<'a>> {
        let mut list = self.programs();
        loop {
            let entry = list.next()?;
            let Ok(entry) = entry else { return None };
            if entry.name == want {
                return Some(entry);
            }
        }
    }

    fn programs(&self) -> manifest::Entries<'a> {
        manifest::Entries::new(self.view).expect("清单头已在 new 时验过")
    }
}

/// 装配：登记整张表，然后按顺序把每条起起来。
///
/// 返最后一条的名字（装配者等它退场；它一走 ⇒ 会话结束）。
///
/// `root` = 与**引导域**那条泊位（配给从那儿来）；`lanes` = 死亡道在本线程表里的句柄
/// （一位服务一条，按单子下标对位），装配时随 `board::attach` 各交一份给板线程。
///
/// **持树者的号与提示之路不在这里**：持树者自己是 `plan` 的第一条（[`Program::holds_tree`]），
/// 认下它那条提示之路的那一步就在下面的循环里——从前它由引导域先起、再当一笔货转授过来，
/// 那一笔已经清掉（它是**服务**，不是引导设施）。
///
/// **身份也在装配里**：身份服务（`plan` 里 [`Eyes::Roster`] 那一条）一放行，本域先认下它交给
/// 生我者的门牌，把**它自己与树**补绑上（它们起来时它还没在）；其后的每一条都在 [`start`] 里、
/// **放行之前**拿到 `derive(root)` + `bind(rep, p)`。**装配者自己（编排域这一枚）不绑**：
/// 它是写名册的那一个，不是被写的那一个。
pub fn assemble<'a>(
    table: &mut Table,
    catalog: &Catalog<'a>,
    plan: &[Program],
    root: &Pier,
    lanes: &[Option<PieToken>],
    machine: &Machine,
) -> Result<Name, Died> {
    // 清单条数与表的格数**同值**（`Table::CAP` = `env::wire::manifest::MAX_PROGRAMS` = 28），
    // 但这里不需要再查一遍：超限清单在 `Catalog::new` 就被 `manifest::Entries::new` 挡掉了。

    // 一、登记：先立账（名字 + 怎么算起来），身子要等真的起了才挂上。
    for p in plan {
        let name = Name::new(p.name).map_err(|_| E_MANIFEST)?;
        table.register(name, p.announce).map_err(|_| E_TABLE)?;
    }

    // 二、逐条起。**顺序即契约**：先起的先就绪，后面的就能向它要东西。
    //
    // `btip` = 板线程那条提示之路在**本线程表里**那一枚；`tree` / `otip` = 持树者的号与它
    // 那条提示之路的同一格。三样都属于本线程这张表，故只能被本线程拿着逐条传
    // （`PieToken` 标着 `!Send + !Sync`，而号也只在铸它的那张表里有意义）。
    let mut btip: Option<PieToken> = None;
    let mut tree: Option<TaskId> = None;
    let mut otip: Option<PieToken> = None;
    // 身份服务那一面：它一起好本域就有，此后每条服务的身份都从它来。
    let mut face: Option<Face> = None;
    // **协调那一帧**要带的两位（名册 / 盟册）：各自那一域起来之后就占上那一格，此后随每一条
    // `attach` 传下去（持树者据此才判得了身份、判得了盟籍）。**定长两格**——这一族只有两双
    // 眼睛（`Eyes`）；门牌**不由这里转授**，各域自己在 `serve_tree` 之后交。
    let mut coord: [(Option<TaskId>, Eyes); 2] = [
        (None, Eyes::Roster),
        (None, Eyes::League),
    ];
    for (i, p) in plan.iter().enumerate() {
        let rep = start(
            table,
            catalog,
            p,
            root,
            tree,
            &mut btip,
            &mut otip,
            face.as_ref(),
            lanes.get(i).copied().flatten(),
            machine,
            coord,
        )?;
        // 它刚把提示之路交给**生我者**（= 本域）⇒ 当场认下来，此后客人上树才有路可走。
        if p.holds_tree {
            tree = Some(rep);
            otip = None;
            operator::host_of(rep, READY_MS, &mut otip).map_err(|why| {
                step(p, why);
                p.died
            })?;
        }
        // **哪一双眼睛**：装配单上那一格说了算（不是拿名字认的——`p.name == "principal"`
        // 那种写法，改个名字就静默失灵；见 `env::assembly::Eyes` 的照实记）。
        //
        // 名册（`principal`）：它一放行，本域就认下它交给生我者的那一枚门牌，再把**前两条**
        // （它自己与树）补绑上——它们起来的时候它还没在，没得绑。
        if let Some(eyes) = p.eyes {
            match eyes {
                Eyes::Roster => {
                    // **协调那一帧**要带的第一格：身份服务自己（名册那一双眼睛）。它那一枚门牌
                    // **由它自己**在 `serve_tree` 之后直接交给持树者（见 `operator/bridge.rs` 的
                    // `COORD` 照实记：装配者转授那一版真机栽在 `coord-ship`），装配者只剩递一格号。
                    coord[0].0 = Some(rep);
                    let f = face_of(rep).ok_or_else(|| {
                        step(p, "no identity face");
                        p.died
                    })?;
                    let mine = f.derive(PrincipalId::ROOT, READY_MS).map_err(|_| {
                        step(p, "derive self");
                        p.died
                    })?;
                    f.bind(rep, mine, READY_MS).map_err(|_| {
                        step(p, "bind self");
                        p.died
                    })?;
                    if let Some(t) = tree {
                        let pt = f.derive(PrincipalId::ROOT, READY_MS).map_err(|_| {
                            step(p, "derive tree");
                            p.died
                        })?;
                        f.bind(t, pt, READY_MS).map_err(|_| {
                            step(p, "bind tree");
                            p.died
                        })?;
                    }
                    face = Some(f);
                }
                // 盟册（`coalition`）：把它的号占进**协调那一帧**的第二格。它的门牌**也是它自己
                // 交的**（同 principal 那一格：`serve_tree` 之后直接交给持树者，见
                // `coalition/server.rs`）——装配者这一侧只递号、不转授。它的 `derive`/`bind`
                // 由上面那条通用路做过（`p.bind`）。
                Eyes::League => coord[1].0 = Some(rep),
            }
        }
    }

    // 三、把最后一条的名字交出去（等它退场 = 等这次会话结束）。
    Name::new(plan.last().map(|p| p.name).unwrap_or("")).map_err(|_| E_PROGRAM)
}

/// 起一条：登记过的账 + 清单里的镜像 + **它的身份** + 它的门闩 + 它的通道。
///
/// 每一步失败都报出**是哪一条、哪一步**（诊断靠这一行，不靠再读一遍代码）。
///
/// **公开**：引导域那一条（编排者）不走本模块的装配单（那一张是编排域私有的），但它要的仍是
/// 同几步——起一条是**同一条路**，不该有两份实现。
///
/// 返**代表的号**（`rep`）：装配者要凭它认下持树者的提示之路（[`assemble`] 那一步）。
#[allow(clippy::too_many_arguments)]
pub fn start<'a>(
    table: &mut Table,
    catalog: &Catalog<'a>,
    p: &Program,
    root: &Pier,
    tree: Option<TaskId>,
    btip: &mut Option<PieToken>,
    otip: &mut Option<PieToken>,
    face: Option<&Face>,
    lane: Option<PieToken>,
    machine: &Machine,
    // **协调那一帧**要带的两格（哪一位域 + 它哪一双眼睛）：只有递门牌那两格填过之后才非空。
    // 它**重复推**（每一位后续客人的 `attach` 都会推一遍）——持树者收到就按位补上，重复只是
    // 再建一次同样的会话（幂等，见 `server.rs` 的 `settle`）。
    coord: [(Option<TaskId>, Eyes); 2],
) -> Result<TaskId, Died> {
    let name = Name::new(p.name).ok().ok_or(E_MANIFEST)?;

    // 一、身子：建域 + 产线程（此刻它一步都还没跑）。
    let entry = catalog.find(p.name).ok_or(E_PROGRAM)?;
    let rep = service::spawn(table, name, entry.elf, entry.kind).map_err(|_| p.died)?;

    // 一之后、二之前：**身份**。装配者给这条服务派生一条号、把它绑到 `rep` 上——**放行之前**
    // 就做完，故服务一起来 `resolve(self)` 就答得出。（身份服务本身与树不走这里：它们起来时
    // 它还没在；那两条由 [`assemble`] 在它放行之后补绑。）
    if let Some(face) = face.filter(|_| p.bind) {
        let mine = face.derive(PrincipalId::ROOT, READY_MS).map_err(|_| {
            step(p, "derive");
            p.died
        })?;
        face.bind(rep, mine, READY_MS).map_err(|_| {
            step(p, "bind");
            p.died
        })?;
    }

    // 二、会话：对端 = **建它那个域的那一枚线程**（= 本域）——它把自己的孔交给"生我者"，
    //     而"生我者"是建域那一枚，**不是刚产出的代表线程**（`rep`）。
    let me = runtime::env::unit::self_id().map_err(|_| {
        step(p, "no self id");
        p.died
    })?;
    // 这座码头的**对端就是客人**（`rep`）——与 `board.rs` 的 `Quay::open(client)` 对称：
    // 两侧各按对方的身份开码头，`seat` 那一枚才发得到它手里，谁都不必猜。
    let mut quay = Quay::open(rep);
    // `marks` = 要逐条认领的记号：**记号就是这条泊位的名字**（`seat` 铸孔时刻上去的），
    // 而客侧装的就是同一个通道名 ⇒ 放行之后本域按它逐条把客人的孔认下来（顺序无关）。
    let mut marks: alloc::vec::Vec<Mark> = alloc::vec::Vec::new();
    marks.try_reserve(p.channels.len()).map_err(|_| {
        step(p, "no room for marks");
        p.died
    })?;
    for ch in p.channels.iter() {
        let ch = Name::new(ch).ok().ok_or(E_MANIFEST)?;
        quay.seat(ch).map_err(|_| {
            step(p, "seat failed");
            p.died
        })?;
        marks.push(Mark::of(ch.as_str()));
    }

    // 三、放行 + 等就绪（有通道的那一条顺带逐条认领）；四、发门闩。
    let ups = if p.channels.is_empty() {
        None
    } else {
        Some(&mut quay)
    };
    service::start(table, name, rep, p.tokens, ups, &marks, READY_MS).map_err(|_| {
        step(p, "start failed");
        p.died
    })?;
    if p.needs.is_some() {
        wire(root, &quay, p, rep, machine).map_err(|why| {
            step(p, why.said());
            p.died
        })?;
    }

    // 五、板：本域是板的宿主 ⇒ 起一枚待客线程，再把客人交出来的那一枚转授给它。
    //     **在 `records` 之后**：板那条路由客人在起来之后自己装（它是问的那一侧），
    //     而它要先收到配给才轮得到板那一问。
    if p.board {
        board::attach(&mut quay, me, rep, READY_MS, btip, lane).map_err(|why| {
            step(p, why);
            p.died
        })?;
    }
    // 六、树：**按需**把这条服务接到持树者那棵树上（[`Program::operator`]）。
    //     **在板之后**：两者各一条路、互不影响；先板后树只为让读数一行行落得整齐。
    if p.operator {
        // 持树者必须**先于**这位客人起（[`Program::holds_tree`]）：提示之路还没认下就没得接。
        let host = tree.ok_or_else(|| {
            step(p, "no tree yet");
            p.died
        })?;
        operator::attach(&mut quay, rep, host, READY_MS, otip, &coord).map_err(|why| {
            step(p, why);
            p.died
        })?;
    }
    Ok(rep)
}

/// 报"哪一条、哪一步没成"。
///
/// 只在失败路径上调：**成功不说话**（装配正常的机器不该刷屏），而失败时这一行决定
/// 还得读几遍代码——所以它报"服务名"与"步骤"两格。
fn step(p: &Program, what: &str) {
    let _ = runtime::env::debug::put(p.name);
    let _ = runtime::env::debug::put(what);
}

/// 递单那一关的失败：**原因就是读数**（[`step`] 印它）。
///
/// 三格分的是"接下来该干什么"：那条路没得发 / 这台机器上没有那一样 / 引导域答了（或没答）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Why {
    /// 客人那条通道还没认下（或者写不进去）——装配次序的事，不是设备的事。
    NoChannel,
    /// **树里没有这一类**——本层翻不出来（不是引导域的答话，那一格是 `Fail::Unknown`）。
    Unplaced,
    /// 引导域那一侧的答复（见 [`protocol::driver::supply::core::Fail`]）。
    Draw(supply::core::Fail),
}

impl Why {
    /// 一行读数的说法。
    pub fn said(self) -> &'static str {
        match self {
            Why::NoChannel => "no channel",
            Why::Unplaced => "class not in tree",
            Why::Draw(_) => "draw failed",
        }
    }
}

/// 递单：**先定坐标**（按类要的那几条读树翻），再向引导域领，最后把那**一段字节原样**
/// 推到客人那条通道上。
///
/// **本域不碰原件**：门闩在引导域手里，它直接授进 `rep` 那张表，回一段"坐标 + 号"的记录；
/// 本域只做一次转投（客人按位次归位，[`protocol::system::grant::each`]）。本层只说
/// "要什么、走哪条通道"——**要什么就是收方那张表**（[`Program::needs`]），一格都不抄。
///
/// **翻坐标是本域唯一解释机器自述的地方**：类（`compatible`）是收方写的，翻成哪一段区是树说的
/// ——两半在这条线上合拢，故那条权威只在这里（`system: <程序> <类> -> <区>` 就是它的读数）。
fn wire(
    root: &Pier,
    quay: &Quay,
    p: &Program,
    rep: env::TaskId,
    machine: &Machine,
) -> Result<(), Why> {
    let Some(needs) = p.needs else {
        return Ok(());
    };
    let Some(ch) = p.channels.first() else {
        return Err(Why::NoChannel);
    };
    let Ok(ch) = Name::new(ch) else {
        return Err(Why::NoChannel);
    };
    let Some(pier) = quay.find(ch) else {
        return Err(Why::NoChannel);
    };
    if !pier.paired() {
        return Err(Why::NoChannel);
    }
    // 条数上限那一格与 `draw` 同一条（单子装不下）：这里先拦，好按定长缓冲逐格填。
    if needs.is_empty() || needs.len() > WANT_MAX {
        return Err(Why::Draw(supply::core::Fail::Local));
    }

    // 一格一格定坐标：类翻成那一段区（读数就是这一行），已经知道坐标的原样落下。
    let mut wants = [Want::NONE; WANT_MAX];
    for (cell, need) in wants.iter_mut().zip(needs) {
        let class = need.class_name();
        *cell = need
            .settle(|class| machine.site_of(class))
            .ok_or(Why::Unplaced)?;
        if let (Some(class), Some(base)) = (class, cell.key().and_then(|key| key.base())) {
            // 新机制要有读数：**类 → 那一段区**（翻译那一手看得见、可复核）。
            let _ = runtime::env::debug::put(&alloc::format!(
                "system: {} {} -> {:#x}",
                p.name,
                class.as_str(),
                base
            ));
        }
    }

    // 一枚一枚要：条数就在那张表里，本层不抄"要几样"。
    let mut ask = [0u8; supply::ORDER_CAP];
    let mut reply = [0u8; supply::REPLY_CAP];
    let records = supply::client::draw(
        root,
        rep,
        &wants[..needs.len()],
        &mut ask,
        &mut reply,
        READY_MS,
    )
    .map_err(Why::Draw)?;
    let said = pier.post(records);
    let _ = runtime::env::debug::put(&alloc::format!(
        "wire: {} bytes, paired={}, post={}",
        records.len(),
        pier.paired(),
        said.is_ok()
    ));
    said.map_err(|_| Why::NoChannel)
}

/// 等一条服务退场（本域等它 = 等这次会话结束）。
pub fn wait_last(table: &mut Table, name: Name) {
    while let Ok(false) = service::watch(table, name, usize::MAX) {}
}

/// 认下身份服务**交给生我者**的那一枚门牌（装配者自己的那一份）。
///
/// 装配期**不必上树查自己起的那一枚**：Server 起手就把门牌那一枚 `ship` 进本域表里，本域按
/// `(开者 = 它, 记号 = entry)` 两格认出来（[`scall::find`] 的两格正判据）。它起手就交，
/// 故这里是**短等**：还没到就隔一拍再问，问到期限为止。
fn face_of(host: TaskId) -> Option<Face> {
    let mut left = READY_MS;
    loop {
        if let Some(entry) = scall::find(host, bcall::ENTRY_MARK) {
            return Face::of(entry).ok();
        }
        if left == 0 {
            return None;
        }
        let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
        left = left.saturating_sub(RETRY_MS);
    }
}

/// 起不来时**报一行**并把原因码交给调用方（调用方 `return` 它，出口那一手在 `entry`）。
///
/// **它不再自己退场**（从前直接调 `room::exit`）：报码这笔账现在由返回值走，与所有别的
/// `main` 同一条路。本域没有会话、没有控制台，调试面是唯一能说话的地方。
pub fn die(died: Died, msg: &str) -> Died {
    let _ = runtime::env::debug::put(msg);
    died
}

/// 等子域就绪/交通道的上限（毫秒）。**必须有界**：子域要是死在头几步，本域不能陪着挂死。
pub const READY_MS: usize = 1000;

/// 装配失败的编号（通用的那几个；按服务分的编号住在各自的装配单旁边）。
pub const E_MANIFEST: Died = 2;
pub const E_PROGRAM: Died = 3;
pub const E_TABLE: Died = 4;
pub const E_OK: Died = 0;
