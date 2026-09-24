//! scenario — **这一景起哪些程序**（装配单本身）。
//!
//! # 为什么装配单单独立一份源（照实记：用户裁定"测试和程序分开"）
//!
//! 这张表原先住 `system/main.rs`——编排的**机器**那一份正文里：15 份产品与 5 台试客
//! （`probe-*`）都在里面（外加按场景选的第二张表）。于是"某一景装哪些台"这件事，写在了产品
//! 程序的正文里。用户的原话是**"为什么 programs 里面的测试和程序混在一起？我不希望这样"**。
//!
//! 故场景归场景，三处各归各位：
//!
//!   - **本文件 = 数据**：一行台 = 一个 [`Program`]（名字 / 宣布 / 通道 / 需求 / 上不上板……）；
//!   - **`system/main.rs` = 机器**：登记、按序起、监督、收尾——它不认识具体哪一台；
//!   - **哪几台进哪张镜像** = `kernel/build.rs` 的 `PRODUCTS` / `PROBES` / `RIGS` + `bins_for`。
//!
//! **照实记（指标与"道"的位次按位耦合）**：装配单的下标就是"道"的位次（`supervise` 按
//! `plan.get(i)` 把道上响的那一位翻回名字）。故两张表必须**各自自洽**——第一版想"表里插一条
//! 空名字的行、调用点过滤"，默认台当场以 `system: manifest bad` 收场（soak 逮住）。

use super::*;

/// 持树者：那棵命名树的服务（`prog-operator`）。**排第一位**——每位上树的客人都要它在。
///
/// 它不宣布、无通道（`Announce::None`：放行即起来）、不上板、不上树（**它就是树**）；起来
/// 之后本域当场把它的提示之路认到手（[`Program::holds_tree`] 那一格）。
const fn tree() -> Program {
    Program {
        name: TREE,
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: false,
        operator: false,
        bind: true,
        holds_tree: true,
        died: E_TREE,
    }
}

/// 身份服务（`prog-principal`，**U 态**）：名册 + 谱系两张表（`protocol::principal`）。
///
/// **排在树之后、其余服务之前**：树先立（门牌要落在它上面），而装配期每一条服务的
/// `derive` + `bind` 都要它已经在——故 [`service::assemble`] 在它放行之后**补绑**它自己与树
/// 那两条（它们起来时身份服务还没在），其后的每一条都在 `Hatch` 之前拿到身份。
///
/// 上板（板看得见它的死）+ 上树（门牌 `/sys/principal`——两段名见
/// [`protocol::principal::DIR`] / [`protocol::principal::NAME`]）。
const fn principal() -> Program {
    Program {
        name: PRINCIPAL,
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: true,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_PRINCIPAL,
    }
}

/// 结盟服务（`prog-coalition`，**U 态**）：横向那张盟籍表（`protocol::coalition`）。
///
/// **紧随身份服务之后**：它是**身份服务的客人**——起手要按名字在树上找到 `/sys/principal`
/// （门牌由 principal 自己那一段落下），每条写原语嵌一次 `Resolve(发送者)`。排在 `principal`
/// 之后是本域能给的唯一次序保证（"就绪"与"上树"不是同一步，故它自己还带一轮有界的重试）。
///
/// 上板（板看得见它的死）+ 上树（门牌 `/sys/coalition`——两段名见
/// [`protocol::coalition::DIR`] / [`protocol::coalition::NAME`]）。
const fn coalition() -> Program {
    Program {
        name: COALITION,
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: true,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_COALITION,
    }
}

/// 线路由者（中断面域）：常驻，要三枚门闩，起来时交回通道；**上板也上树**。
///
/// - 上板（`board: true`）只为让板看得见本域的死（编排域监督的事件源）——**它不挂牌子**了。
/// - 上树（`operator: true`）：它的服务入口落在 `/device/router`（[`protocol::driver::DIR`]）
///   ——**这就是它的门牌**，按名找它的人从树上问（`guest` 那一趟）。
const fn router() -> Program {
    Program {
        name: "router",
        announce: Announce::Channel,
        tokens: &[],
        channels: &["records"],
        needs: Some(router_needs::WANTS),
        board: true,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_ROUTER,
    }
}

/// 串口驱动：常驻，要一枚门闩（**按类 `ns16550a` 要**——本机上是 `serial@10000000`），
/// 起来时交回通道；上板、上树。
///
/// **它排在线路由者之后**：控制器先就位，线再开闸（闸门归持有设备的那一台，见该域头注）。
/// **上树是"按名找人"**：它从树上找到 `/device/router` 那位、把本域那条线**登记**下来
/// ——线从此归它，投递也到得了它（`line::client` 那几手）。
const fn uart() -> Program {
    Program {
        name: "uart",
        announce: Announce::Channel,
        tokens: &[],
        channels: &["records"],
        needs: Some(uart_needs::WANTS),
        board: true,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_UART,
    }
}

/// 实时钟驱动：**第二台真设备**（**按类 `google,goldfish-rtc` 要**——本机上是 `rtc@101000`，
/// 11 号线）——兼**报时服务**（门牌 `/device/rtc`）：
/// 客人问时间就地答；客人约一个时刻就占住那一格并武装设备，到点把"那一声"推回客人手里。
///
/// 上树两趟（`operator: true`）：落自己那块门牌 + 按名找线路由者。上板只为让板看得见它的死
/// （它常驻）。两台设备驱动紧跟在树之后：控制器先就位，线才有人接。
const fn rtc() -> Program {
    Program {
        name: "rtc",
        announce: Announce::Channel,
        tokens: &[],
        channels: &["records"],
        needs: Some(rtc_needs::WANTS),
        board: true,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_RTC,
    }
}

/// 客人：从**树上**找到 `router`（`/device/router`）、说一句、把答话带回来。
///
/// 它什么都不交回（`Announce::None`：本域不等它），故**上板那一格由 `board` 那一支负责**
/// ——本域等的是它那条板路接上（[`board::attach`] 的第 2 步），不是它说了什么。
///
/// **两条目录都走**：自己那块牌子仍挂板（`REGISTER` + 退场那句 `EVICT` 只有它在用），
/// 而"找别人"走树（`operator: true`）——按名找服务从此归树，板管生死。
const fn guest() -> Program {
    Program {
        name: "guest",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: true,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_GUEST,
    }
}

/// 过客：起来、挂一个名字、**直接死**（不说再见）。
///
/// 板上那两本账的"死"判据（"**那一枚入口还答得出吗**"，`VestedBy` = `Reserve` 那一格）就是为它
/// 存在的读数：它不说 `EVICT`，故只有"看出来的"那一档收得掉它。
const fn passer() -> Program {
    Program {
        name: "passer",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: true,
        operator: false,
        bind: true,
        holds_tree: false,
        died: E_PASSER,
    }
}

/// 房客：**起来、占一条线、直接死**——线那本账探活的读数（见 `programs/src/user/lodger`）。
///
/// 它要一台 virtio（**按类 `virtio,mmio` 要**——本机上八台同类，编排域取 `reg` 首址最小的那台
/// = `virtio_mmio@10001000`，**一条没人要的线**）：**它真持有那台设备**，
/// 却从不映视图、不碰寄存器——占住线之后一句话不说就走。路由者那边靠 `sweep` 收掉它
/// （`router: vacate line=1`）。**照实记**：它从前占的是 11 号线（那时钟），第二台设备驱动
/// 上来之后那条线有主了。
///
/// **有通道就得等通道**（`Announce::Channel`）：配给经 `records` 那条通道发，而通道要先被本域
/// 认下来（`server::ready` 里那一手）——写成 `None` 就没人认领它，`wire` 当场失败（实测踩过）。
///
/// 它不上板（`board: false`）：板上那本账与本域无关，本域要喂的是**线那本账**；它死时
/// 编排域照样看得见（每条服务一条死亡道）。
const fn lodger() -> Program {
    Program {
        name: "lodger",
        announce: Announce::Channel,
        tokens: &[],
        channels: &["records"],
        needs: Some(lodger_needs::WANTS),
        board: false,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_LODGER,
    }
}

/// 客人：`/device/rtc` 那面服务的第一位用家——问时间、约一个时刻、等到那一声再退场。
///
/// 它**不要门闩**（`needs: None`）：那台时钟归 `rtc` 持有（`ONLY` 是资源事实），它拿到的只是
/// 树上那枚门牌孔的副本。失败域那两格也各走一趟（见 `programs/src/user/sleeper.rs`）。
///
/// `Announce::None`（本域不等它说话）+ 上板：与 `guest` 同一档——"它死了"由板那条道看出来。
/// 它排在 `echo` 之前：`echo` 必须是最后一条（本域等它退场收场）。
const fn sleeper() -> Program {
    Program {
        name: "sleeper",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: true,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_SLEEPER,
    }
}

/// 调试回显：只走调试面、不要门闩、不交通道。**但它上板也上树**：
///
/// - 上板（`board: true`）只为让板**看得见它的死**：它退场时开的那几枚孔随退出钩子封印
///   ⇒ 板当场看出"客人没了" ⇒ 推一格死亡通知给装配者。本域的监督事件源就是这一条。
/// - 上树（`operator: true`）是**树上第一位客人**：它把本域的入口挂到树上、再查回来取一枚
///   （`echo.rs::trip` 那几行）——树的载体因此有一条**跑在机器上的读数**。**照实记**：今天树上
///   的真门牌是驱动族那块 `/device/router`（`protocol::driver::DIR`），而本域仍落在自己名下
///   ——服务不上 `/device`。
const fn echo() -> Program {
    Program {
        name: "echo",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: true,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_ECHO,
    }
}

/// 主体（`prog-subject`，U 态）：身份服务的第一位真客人——问自己是谁、验三态、向下派生、
/// 再越权趟一次（读数见 `programs/src/user/subject.rs` 头注）。
///
/// **不上板**（同 `lodger`）：它只做一件事，生死那本账与它无关。**排在 `echo` 之前**：
/// `echo` 必须是最后一条（本域等它退场收场）。
const fn subject() -> Program {
    Program {
        name: "subject",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: false,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_SUBJECT,
    }
}

/// 盟友（`prog-member`，U 态）：结盟服务的第一位真客人——立两枚盟、进进出出、验幂等与第三态，
/// 再用派生的第二条身份验"同一枚盟里有两位"（读数见 `programs/src/user/member.rs` 头注）。
///
/// 它**要两面门牌**（都在树上按名字找）：`/sys/coalition` 是主角，`/sys/principal` 用来派生
/// 第二条身份（K1 那条"键 = 身份"的定理要在机器上读出来）。
///
/// **不上板**（同 `subject` / `lodger`）：它只做一件事，生死那本账与它无关。**排在 `echo`
/// 之前**：`echo` 必须是最后一条（本域等它退场收场）。
const fn member() -> Program {
    Program {
        name: MEMBER,
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: false,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_MEMBER,
    }
}

/// 负证客人（`prog-probe-denied`，U 态）：**没有身份**的任务去撞树的门。
///
/// 它是门禁那条"没绑身份 ⇒ 拒绝"判据在**真机上**的反例——`bind: false` 让装配者**不绑它**
/// （其余每一条都绑），于是它 `resolve(self)` 答 `None`，树那一问答 `DENIED`。它随后再
/// `seek` 一次，证"拒绝不是换绑"。读数见 `harness/src/probe_denied.rs` 头注。
///
/// **排在 `echo` 之前**（`echo` 必须最后一条）；它自己退场，不常驻。
const fn probe_denied() -> Program {
    Program {
        name: "probe-denied",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: false,
        operator: true,
        bind: false,
        holds_tree: false,
        died: E_PROBE,
    }
}

/// 负证客人（第二种，`prog-probe-owner`，U 态）：**有身份**地去顶别人声明归自己的一格。
///
/// 与 `probe-denied` 分工：那一台撞"**没身份**"（第一道门），本台撞"**那一格归谁**"
/// （`land` 那一格里 `mine = true`）。它**照常绑身份**（`bind` 缺省 `true`）——否则量到的是
/// 同一道门。**照实记**：这一句原写的是 `Rule::Owner`，而那个变体在「两轴分家」那一刀就没了
/// （改那一轴退成 `call::Rule` 那一格 `mine: bool`）。
const fn probe_owner() -> Program {
    Program {
        name: "probe-owner",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: false,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_PROBE_OWNER,
    }
}

/// 会死的持有者（`prog-probe-lease`，U 态）：落一块**声明归自己**的门牌（`/sys/lease`）
/// 然后**直接死**。
///
/// 它与 `probe-owner` 是一对：那一台证"活着的持有者顶不掉"，本台留下的那块名字证
/// "**主人不在场 ⇒ 那一格重新可落**"（否则命名空间里会留一块没人能改的墓碑）。
const fn probe_lease() -> Program {
    Program {
        name: "probe-lease",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: false,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_PROBE_LEASE,
    }
}

/// 规矩那一格的证客（`prog-probe-rule`，U 态）：有身份的一台把「用」那一轴的五格规矩连同三格
/// 边角（一块 `Pane` 当门牌、剪掉的门牌、自己声明归属的那一格）落到 `/sys/rule` 下，先以自己试，
/// 再 `adopt` 一条子身份试一遍——**逐格读数与判据见那份源码的头注**（那里是唯一权威，
/// 这里只留"它排在哪、为什么"）。
///
/// **排在 `probe-rule-other` 之前**：那一位要按名字去找那几格（它自己带一轮有界重试）。
/// 它**照常绑身份**（`bind` 缺省 `true`）——否则量到的会是 `probe-denied` 那一格。
const fn probe_rule() -> Program {
    Program {
        name: "probe-rule",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: false,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_PROBE_RULE,
    }
}

/// 另一位客人（`prog-probe-rule-other`，U 态）：**有身份**地去用别人立了规矩的那两格。
///
/// 它证的是门禁**第二道门**的反例（"这一格不给你"），与 `probe-denied` 那道"你没身份"分工。
const fn probe_rule_other() -> Program {
    Program {
        name: "probe-rule-other",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: false,
        operator: true,
        bind: true,
        holds_tree: false,
        died: E_PROBE_OTHER,
    }
}

/// **装配单**：本域按这个顺序起服务。
///
/// 持树者（`operator`）**排第一**：它是**服务**，但每位上树的客人都要它在——起来之后本域
/// 当场把它那条提示之路认到手（[`service::assemble`] 的第二段）。
///
/// 身份服务（`principal`）**紧随其后**：装配期每一条服务的 `derive` + `bind` 都要它在
/// （[`service::assemble`] 在它放行之后补绑它自己与树，其后的每一条都在放行前拿到身份）。
///
/// 结盟服务（`coalition`）**跟在身份服务之后**：它是身份服务的客人（起手按名字找
/// `/sys/principal`），故只能在它之后起——这也是本域能给的唯一次序保证（那一台自己还带一轮
/// 有界的重试，见 [`coalition`] 那一格）。
///
/// `echo` **必须在最后**：[`service::assemble`] 返 [`PLAN`] 的最后一条，本域等它退场
/// ——那正是"读到一行 `exit` 才收场"的那一格。**它得是"上板"的那一条**（`board: true`），
/// 板才看得见它的死。照实记：试过把探针放最后，机器**不再停机**——探针不上板，那一等没人应。
///
/// 三台驱动紧跟在身份服务之后、其余之前：控制器先就位，线再开闸（`uart` / `rtc` 持有那两台设备）。
/// `sleeper` 排在 `lodger` 之后、`subject` 之前：它要找的那块门牌 `/device/rtc` 由 `rtc` 落。
pub const PLAN: &[Program] = &[
    tree(),
    principal(),
    coalition(),
    router(),
    uart(),
    rtc(),
    guest(),
    passer(),
    lodger(),
    sleeper(),
    subject(),
    member(),
    probe_denied(),
    probe_lease(),
    probe_owner(),
    probe_rule(),
    probe_rule_other(),
    echo(),
];
