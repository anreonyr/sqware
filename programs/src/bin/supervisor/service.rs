//! service — **装配策略表**：这台机器该起哪些服务、用什么起、起了之后跟它说什么。
//!
//! 这张表是"装配"这件事**唯一**的地方：名字、怎么算起来、起跑前塞什么、开哪几条通道、
//! 要哪些门闩、上不上板、上不上树、失败了报哪个号——八样以前散在八个地方（`announce_of`、两个
//! `const`、代码里隐含的通道名、`needs`、`board::attach` 的调用点、各处期限、两套 `E_*`），
//! 现在合成一处：
//!
//! ```text
//!   PLAN  └─ Program { name, announce, tokens, channels, needs, board, operator, died }
//! ```
//!
//! 想加第三个服务：在 [`PLAN`] 里加一行，**`main` 一个字都不用改**。

use env::Name;
use env::wire::manifest;
use protocol::session::Quay;
use protocol::system::service::{self, Announce, Grant, Table};
use runtime::core::port::ship;
use runtime::env::mail::{NolePie, PolePie};
use runtime::env::room::exit_with;

use super::board;
use super::needs::{self, Kind};
use super::operator;
use super::pairing::Root;

/// 装配失败的编号：指"死在装配的哪一步"（沿用旧树那套小整数编号的意思）。
pub type Died = usize;

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
    pub needs: Option<&'static [needs::Need]>,
    /// 要不要板那条路（[`board::attach`]）。
    ///
    /// **它不是 `channels` 里的一行**：那几条是"放行前先装好、起来时交回"，而板那条路由
    /// **客人在起来之后自己装**（客人才是"问"的那一侧），故本域是等它，不是先装。
    /// 两端共用这一格：`true` ⇒ 它一定调 [`board::open`]（不然本域要白等一期）。
    pub board: bool,
    /// 要不要树那条路（[`operator::attach`]）。
    ///
    /// 与 [`Program::board`] 同一个形状、同一格位置（"两端共用"）：`true` ⇒ 它一定调
    /// [`operator::open`]。**按需发**——拿到这条路的服务，就能动整棵树（本正文不做权限
    /// 判断，owner 归 Principal），故只有确实要用的那几条打上它。
    pub operator: bool,
    /// 装配死在这一条时报哪个号。
    pub died: Died,
}

/// 持树者那一条在 [`PLAN`] 里的名字。**两端共用**：装配单按它对位，`start` 按它认出
/// "这一条就是持树者"（把它交回来的号记下来给后面几条用）。
pub const OPERATOR: &str = "operator";

/// **装配单**：本域按这个顺序起服务。
///
/// `operator` **必须在最前**：后面几条按需接上它（[`Program::operator`]），而"接上"要它
/// 已经把提示孔交回来——先起它，后面那几条装路时它早就在了。
///
/// `echo` **必须在最后**：[`assemble`] 返 [`PLAN`] 的最后一条，root 的 `wait_last` 等它
/// ——那正是"读到一行 `exit` 才收场"的那一格。
pub const PLAN: &[Program] = &[tree(), plic(), guest(), passer(), echo()];

/// 命名树的服务（`prog-operator`，U 态、独立域）：它自己要什么？**什么也不要**——不交通道、
/// 不要门闩、不上板（它不给人挂牌子，它自己就是那棵树）。它只把提示孔交给本域。
///
/// **它是唯一一条 `operator: false` 而持有树的服务**：本域把它起起来之后，就按需把别的
/// 服务接到它那棵树上。
const fn tree() -> Program {
    Program {
        name: OPERATOR,
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: false,
        operator: false,
        died: E_OPERATOR,
    }
}

/// 中断面域：常驻，要四枚门闩，起来时交回通道，并挂上板。
const fn plic() -> Program {
    Program {
        name: "plic",
        announce: Announce::Channel,
        tokens: &[],
        channels: &["records"],
        needs: Some(needs::PLIC),
        board: true,
        operator: false,
        died: E_PLIC,
    }
}

/// 客人：按名字找到 `plic`、说一句、把答话带回来。
///
/// 它什么都不交回（`Announce::None`：本域不等它），故**上板那一格由 `board` 那一支负责**——
/// 本域等的是它那条板路接上（[`board::attach`] 的第 2 步），不是它说了什么。
const fn guest() -> Program {
    Program {
        name: "guest",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: true,
        operator: false,
        died: E_GUEST,
    }
}

/// 过客：起来、挂一个名字、**直接死**（不说再见）。
///
/// 板上那两本账的"死"判据（"**那一枚入口还答得出吗**"，`Probe` = `Reserve` 那一格）就是为它
/// 存在的读数：它不说 `DISMISS`，故只有"看出来的"那一档收得掉它。
const fn passer() -> Program {
    Program {
        name: "passer",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: true,
        operator: false,
        died: E_PASSER,
    }
}

/// 调试回显：只走调试面，不要门闩、不交通道。**但它上板也上树**：
///
/// - 上板（`board: true`）只为让板**看得见它的死**：它退场时开的那几枚孔随退出钩子封印
///   ⇒ 板当场看出"客人没了" ⇒ 推一格死亡通知给装配者。`root` 的监督事件源就是这一条
///   （见 `root/main.rs::supervise`）。
/// - 上树（`operator: true`）是**第一位真客人**：它把本域的入口挂到树上、再查回来取一枚
///   （`echo.rs::trip` 那几行）——树的载体因此有一条**跑在机器上的读数**，而不是只过了编译。
const fn echo() -> Program {
    Program {
        name: "echo",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: true,
        operator: true,
        died: E_ECHO,
    }
}

/// 装配失败编号（按服务分：看日志就知道死在哪儿）。
pub const E_BOOT: Died = 1;
pub const E_MANIFEST: Died = 2;
pub const E_PROGRAM: Died = 3;
pub const E_TABLE: Died = 4;
pub const E_PLIC: Died = 5;
pub const E_ECHO: Died = 6;
pub const E_GUEST: Died = 7;
pub const E_PASSER: Died = 8;
pub const E_OPERATOR: Died = 9;
pub const E_OK: Died = 0;

/// 装配：登记整张表，然后按顺序把每条起起来。
///
/// 返最后一条的名字（本域等它退场；它一走 ⇒ 本域退出 ⇒ 级联 ⇒ 停机）。
///
/// `lanes` 是**死亡道**在装配者表里的那一把句柄（一位服务一条，按 `PLAN` 下标对位）：
/// 装配时随 `board::attach` 各交一份给板线程，装配完由 `root` 拿去监督
/// （见 `root/main.rs::supervise`）。它**不是**本函数的产物，只是借道穿过。
pub fn assemble(
    table: &mut Table,
    boot: &Root,
    lanes: &[Option<env::PieToken>],
) -> Result<Name, Died> {
    // 清单条数与表的格数**同值**（`Table::CAP` = `env::wire::manifest::MAX_PROGRAMS` = 16），
    // 但这里不需要再查一遍：超限清单在 `Root::take` 就被 `manifest::Entries::new` 挡掉了，
    // 到不了本函数。
    //
    // **照实记**：旧注写的是"清单条数不能超过表的格数……编译期常量断言过"，还跟着一个
    // `boot.programs().size_hint().0 > MAX_PROGRAMS` 的守卫——**两条都不成立**：全树没有
    // 那条 `const` 断言（grep `MAX_PROGRAMS` 只命中 manifest 与本文件），而 `Entries` 只
    // 实现了 `next`、没覆写 `size_hint` ⇒ 它恒返 `(0, None)`，那个守卫永不成立。

    // 一、登记：先立账（名字 + 怎么算起来），身子要等真的起了才挂上。
    for p in PLAN {
        let name = Name::new(p.name).map_err(|_| E_MANIFEST)?;
        table.register(name, p.announce).map_err(|_| E_TABLE)?;
    }

    // 二、逐条起。**顺序即契约**：先起的先就绪，后面的就能向它要东西。
    // `tip` = 板线程那条提示之路在**本线程表里**的那一枚；`otip` = 持树者那条提示之路的
    // 同一格。两样都属于本线程这张表，故只能被本线程拿着逐条传（`PieToken` 标着
    // `!Send + !Sync`）。`host` = 持树者的号（起它的那一刻记下来，后面按需用它）。
    let mut tip: Option<env::PieToken> = None;
    let mut otip: Option<env::PieToken> = None;
    let mut host: Option<env::TaskId> = None;
    for (i, p) in PLAN.iter().enumerate() {
        start(
            table,
            boot,
            p,
            &mut tip,
            &mut otip,
            &mut host,
            lanes.get(i).copied().flatten(),
        )?;
    }

    // 三、把最后一条的名字交出去（等它退场 = 等这次会话结束）。
    Name::new(PLAN.last().map(|p| p.name).unwrap_or("")).map_err(|_| E_PROGRAM)
}

/// 起一条：登记过的账 + 清单里的镜像 + 它的门闩 + 它的通道。
///
/// 每一步失败都报出**是哪一条、哪一步**（诊断靠这一行，不靠再读一遍代码）。
fn start(
    table: &mut Table,
    boot: &Root,
    p: &Program,
    tip: &mut Option<env::PieToken>,
    otip: &mut Option<env::PieToken>,
    host: &mut Option<env::TaskId>,
    lane: Option<env::PieToken>,
) -> Result<(), Died> {
    let name = Name::new(p.name).ok().ok_or(E_MANIFEST)?;

    // 一、身子：建域 + 产线程（此刻它一步都还没跑）。
    let entry = find(boot, p.name).ok_or(E_PROGRAM)?;
    let rep = service::spawn(table, name, entry.elf, entry.kind).map_err(|_| p.died)?;
    // 这一条就是持树者 ⇒ 把它的号记下来：后面的服务要按需接到它那棵树上
    // （`Accord` 的目的地就是一个号，故"接上"只需要这个号 + 一条答话路）。
    if p.name == OPERATOR {
        *host = Some(rep);
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
    let mut marks: alloc::vec::Vec<Name> = alloc::vec::Vec::new();
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
        marks.push(ch);
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
        wire(&quay, p, boot, rep).map_err(|_| {
            step(p, "wire failed");
            p.died
        })?;
    }

    // 五、板：本域是板的宿主 ⇒ 起一枚待客线程，再把客人交出来的那一枚转授给它。
    //     **在 `records` 之后**：板那条路由客人在起来之后自己装（它是问的那一侧），
    //     而它要先收到配给才轮得到板那一问。
    if p.board {
        board::attach(&mut quay, me, rep, READY_MS, tip, lane).map_err(|why| {
            step(p, why);
            p.died
        })?;
    }

    // 六、树：**按需**把这条服务接到持树者那棵树上（[`Program::operator`]）。
    //     **在板之后**：两者各一条路、互不影响；先板后树只为让读数一行行落得整齐。
    if p.operator {
        let Some(host) = *host else {
            step(p, "no tree host");
            return Err(p.died);
        };
        operator::attach(&mut quay, rep, host, READY_MS, otip).map_err(|why| {
            step(p, why);
            p.died
        })?;
    }
    Ok(())
}

/// 报"哪一条、哪一步没成"。
///
/// 只在失败路径上调：**成功不说话**（装配正常的机器不该刷屏），而失败时这一行决定
/// 还得读几遍代码——所以它报"服务名"与"步骤"两格。
fn step(p: &Program, what: &str) {
    let _ = runtime::env::debug::put(p.name);
    let _ = runtime::env::debug::put(what);
}

/// 发货：按这张表的需求单把门闩交出去，再把记录推进那条通道。
///
/// **编码住 [`pairing`]**：这里只说"发什么、走哪条通道"，不说字节长什么样。
fn wire(quay: &Quay, p: &Program, boot: &Root, rep: env::TaskId) -> Result<(), ()> {
    if p.needs.is_none() {
        return Ok(());
    }
    let Some(ch) = p.channels.first() else {
        return Err(());
    };
    let Ok(records) = Name::new(ch) else {
        return Err(());
    };
    let Some(pier) = quay.find(records) else {
        return Err(());
    };
    if !pier.paired() {
        return Err(());
    }

    // 一枚一枚交出去：`ship` 返的是**种在它表里**的句柄（它正是靠那个号说话）。
    // 条数由需求单自己算（`pack` 内部按 `needs::PLIC` 逐条走），本层不抄"要几样"。
    let bytes = match boot.pack(|src, need| {
        let at = match need.kind {
            Kind::Pole => ship(&PolePie::from_token(src), rep, need.access, need.policy),
            Kind::Nole => ship(&NolePie::from_token(src), rep, need.access, need.policy),
        };
        at.ok().map(|to| to.seed())
    }) {
        Some(bytes) => bytes,
        None => {
            return Err(());
        }
    };
    let said = pier.post(&bytes);
    let _ = runtime::env::debug::put(&alloc::format!(
        "wire: {} bytes, paired={}, post={}",
        bytes.len(),
        pier.paired(),
        said.is_ok()
    ));
    said.map_err(|_| ())
}

/// 从清单里挑出这个程序。
fn find(boot: &Root, want: &str) -> Option<manifest::Entry<'static>> {
    let mut list = boot.programs();
    loop {
        let entry = list.next()?;
        let Ok(entry) = entry else { return None };
        if entry.name == want {
            return Some(entry);
        }
    }
}

/// 等一条服务退场（本域等它 = 等这次会话结束）。
pub fn wait_last(table: &mut Table, name: Name) {
    while let Ok(false) = service::watch(table, name, usize::MAX) {}
}

/// 起不来时报一行并退出。本域没有会话、没有控制台，调试面是唯一能说话的地方。
pub fn die(died: Died, msg: &str) -> ! {
    let _ = runtime::env::debug::put(msg);
    exit_with(died)
}

/// 等子域就绪/交通道的上限（毫秒）。**必须有界**：子域要是死在头几步，本域不能陪着挂死。
const READY_MS: usize = 1000;
