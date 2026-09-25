//! 默认那一景的读数门（搬自 `scripts/soak.sh`——**那一刀之后它已删**，判据一字未改）——**判据住这里**，
//! `tests/soak.rs` 那一层只管起机与报数。
//!
//! # 判据（两条一起）
//!
//!   1) 读数里出现 `task: all tasks exited, system halted`（关机的唯一判据）；
//!   2) 那批启动 / 装配读数 + 一条**关系**判据 + `[case]` 那一族的**逐台基线**都还在，
//!      且**读数表**（`READINGS`）逐条兑现——每个前缀都在表里声明过、`auto` 档每一行都有人判、
//!      `narrative` 档没判的行落在声明的形状里（棘轮）、`manual` 档写明理由。
//!
//! # 照实记（喂键那一格：等标记，不按钟表）
//!
//! 这一门**丢过两轮假红**，两轮都红在"喂键与启动期赛跑"上：按钟表喂的那一版，启动慢的那一
//! 轮里 `exit` 落在"读口还没就绪"的窗口里 ⇒ 门报"无停机行"，而机器其实好好的。定下来的法是
//! **等标记再喂**：轮询本轮日志等 `member: done`（探针收尾那一句），等到了才喂 `exit`；
//! 25 秒还没等到就**兜底照喂**——那一支注定红在判据上，不是红在超时上。
//!
//! **照实记（第二条 `exit` 是保险）**：等标记之后本轮往往在第一条就收尾，第二条会撞上断开的
//! 管道——那是**预期的收尾**，不是错（旧脚本为此专门把喂键器的 stderr 闭掉）。这里写不进去
//! 就写不进去：`writeln!` 的错误被忽略，而那正说明机器已经收场了。
//!
//! # 照实记（为什么统一关掉 `icount`）
//!
//! `boot.nu` 的默认是 `-icount auto,sleep=on`：按宿主时间给 vCPU 记账、让它睡够虚拟额度 ⇒
//! **WFI 里的核被 IPI 叫醒要等额度（实测毫秒级）**。这一门与验收门、压测台、忙机台、共享组台
//! 此前都关着跑，而**台子与忙机台没关** ⇒ 两边读数**不可比**（照实记：rig A 的 `starved` 在
//! icount 开时是 317/328，关掉后是 1~3/328；同一颗 ELF、同一条命，只差这一个开关）。
//! `run` 对机器那一台**统一置空**，这条纪律从此不用每一门各自记得。
//!
//! # 照实记（为什么要有基线）
//!
//! 这一门原先只查"这一族在不在"，于是**少跑**成了唯一看不见的坏消息：探针那五台的用例是
//! `[case]` 汇总行报的，某一台少登记一例、或者根本没起来，断言照绿。故 `[case]` 那一族是
//! **逐台两条**（登记了几例 / 跑完几例）——改判据就改这里的数，那一改会出现在 diff 里。
//!
//! # 照实记（`GATE_ROUNDS`）
//!
//! 旧脚本默认连跑 **10 轮**。收进门之后默认 **1 轮**（一条 `#[test]` 该是有界的），连跑由
//! `GATE_ROUNDS=<n>` 给。这一格是搬的时候定的口径，不是自明的——记在这里好改。

use crate::*;
use std::time::Duration;

pub const HALT: &str = "task: all tasks exited, system halted";

/// 喂键：等探针收尾再喂 `exit`；上限到了**照喂**（那一支注定红在判据上，不是红在超时上）。
pub const FEED_AFTER: &str = "member: done";

pub fn feed() -> Schedule {
    Schedule::OnMark {
        mark: FEED_AFTER,
        within: Duration::from_secs(25),
        then: vec!["exit".to_string(), "exit".to_string()],
    }
}

pub const MARKS: &[Mark] = &[
    // ── 机器自己那本账的结论：**恰好一次**（用户裁定"路三"）─────────────────────
    //
    // **照实记（为什么是"恰好一次"，以及为什么只有这 13 条）**：下面标 `Once` 的那些，判的是
    // **机器自己那本账**——`router` 的 `told`（一线报一次）、line claim（一条线认一次）、闹钟
    // （响一次）——而程序**已经用那本账决定打不打这一行** ⇒ 门读的是那本账的**结论**，不是新发明。
    //
    // 体量（**12 份验收现场**，`target/gate/*.log` 与 `target/pre-fix/*.log`）：这 13 条
    // **每一份都恰好一次**。而**没有**收紧那几族是对的：
    //
    //   · `uart: rang …`（**3~8 行**）与 `router: exhaust line=10`（**4~8 行**）——条数由时序定，
    //     **没有"对的值"**；
    //   · `rang 条数 == exhaust 条数` **不成立**（**29/30**：`product-1790248191.log` 里 rang=3
    //     而 exhaust=4）⇒ 那条关系钉不得；
    //   · `uart: rang … out=` **不该**钉成"∀ 都是 true"：`out=false` 是**合法状态**（读口没了、
    //     字节没人收），钉它就是把**场景**（喂键刚好赶在 echo 死之前）当成机器性质。
    //
    // **照实记（`router: exhaust line=11` 为什么也在里面）**：验收现场 **8/8 恰好一次**（闹钟响
    // 一次 ⇒ rtc 排空一次）；那 4 份"0 次"的是**产品镜像**（`sleeper` 不在，没人定闹钟）——
    // 产品那一门判的正是它的**缺席**（`crates/gate/tests/product.rs` 的 `ABSENT`）。
    Mark::Literal("router: device_count=95 ctx=1"),
    Mark::Literal("uart: ier=rx at=0x10000000"),
    Mark::Literal("system: router sifive,plic-1.0.0 -> 0xc000000"),
    Mark::Literal("system: uart ns16550a -> 0x10000000"),
    Mark::Literal("system: rtc google,goldfish-rtc -> 0x101000"),
    Mark::Literal("system: lodger virtio,mmio -> 0x10001000"),
    Mark::Literal("devices: 21 handed to root"),
    Mark::Shape("^board: (bye tid=[0-9]+ names=[0-9]+ occupied=[0-9]+ swept=[0-9]+|swept n=[0-9]+ occupied=[0-9]+)[[:space:]]*$"),
    Mark::Shape("wire: [0-9]+ bytes, paired=true, post=true"),
    Mark::Literal("note: passer: gone"),
    Mark::Literal("system: done"),
    Mark::Literal("root: block n=21 region=19 dtb=1 irq=1 bad=0"),
    Mark::Once("^router: line 10 = serial@10000000$"),
    Mark::Literal("uart: rang n="),
    Mark::Once("^router: line 11 = rtc@101000$"),
    Mark::Once("^rtc: line occupied$"),
    Mark::Once("^rtc: armed at=[0-9]+ ier=[0-9]+ alarm=[0-9]+$"),
    Mark::Once("^router: line=11$"),
    Mark::Once("^rtc: rang n=1 now=[0-9]+$"),
    Mark::Once("^router: exhaust line=11$"),
    Mark::Once("^router: vacate line=1$"),
    Mark::Once("^router: lane dropped line=1 pies=21$"),
    Mark::Once("^router: line 1 = virtio_mmio@10001000$"),
    Mark::Once("^router: line=10$"),
    Mark::Literal("router: exhaust line=10"),
    Mark::Once("^uart: line occupied$"),
    Mark::Literal("echo: console=true"),
    Mark::Shape("^coalition: tree part=0 dir=[0-9]+ land=0 find=0 got=true entry=[0-9]+ plate=[0-9]+ pname=coalition[[:space:]]*$"),
    Mark::Shape("^principal: tree part=0 dir=[0-9]+ land=0 find=0 got=true entry=[0-9]+ plate=[0-9]+ pname=principal[[:space:]]*$"),
    Mark::Literal("subject: done"),
    Mark::Literal("probe: tree land="),
    // **照实记（这一行跟着读数改过一次）**：原写 `probe-denied: denied as expected`，而
    // `probe_denied.rs` 的收尾读数在 "删掉变异门" 那一刀（`68e11c8`）里缩短成
    // `probe-denied: denied` ⇒ 表没跟着改，soak 从那时起一直是红的（那一刀的判据只记了
    // "宿主门 PASS"，而 soak 要起 QEMU，没跑）。读数表就该跟着读数走。
    Mark::Literal("probe-denied: denied"),
    Mark::Literal("probe-owner: tree land="),
    Mark::Literal("probe-owner: owner rule held"),
    Mark::Literal("probe-lease: tree land="),
    Mark::Literal("probe-lease: landed, leaving"),
    Mark::Literal("probe-owner: lease land="),
    Mark::Literal("probe-rule: tree part="),
    Mark::Literal("probe-rule: the rules held"),
    Mark::Literal("probe-other: tree is="),
    Mark::Literal("probe-rule-other: all three denied as expected"),
    Mark::Shape("^\\[case\\] probe-denied: 2 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] probe-denied: cases 2 ok 2 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] probe-owner: 3 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] probe-owner: cases 3 ok 3 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] probe-rule-other: 3 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] probe-rule-other: cases 3 ok 3 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] probe-lease: 1 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] probe-lease: cases 1 ok 1 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] probe-rule: 16 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] probe-rule: cases 16 ok 16 fail 0[[:space:]]*$"),
    // 上界那一格的证客（**A 那一刀的读数**）：一页 + 1 被拒的那一行，与"界守住了"那一句。
    Mark::Literal("probe-bound: push="),
    Mark::Literal("probe-bound: bound held"),
    Mark::Shape("^\\[case\\] probe-bound: 3 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] probe-bound: cases 3 ok 3 fail 0[[:space:]]*$"),
    Mark::Absent("operator: two asks"),
    // 判不了那一格的**为什么**：`probe-rule` 那两格（`at_pane` / `gone_door`）必落在 `9`，
    // 而它们各自走 `Court::opens` 的一条臂 ⇒ 这两行读数**必然**在。装配期不产它，产品镜像
    // 也不产（没有一位客人写 `Rule::Opens`）。
    Mark::Shape("^operator: opens (gone|pane|sealed) n=[0-9]+$"),
    Mark::Literal("probe-other: tree is=8 under=8 foreign=8"),
    Mark::Literal("probe-rule-other: all three denied as expected"),
    Mark::Literal("echo: list root=0,3"),
    Mark::Literal("echo: list names=sys,device"),
    Mark::Literal("echo: list device=4,5,6"),
    Mark::Shape("^timer: late_n=[0-9]+ late_max_ms=[0-9]+ late_avg_ms=[0-9]+ late_max_tick=[0-9]+ traps=[0-9]+ tocks=[0-9]+ mutes=[0-9]+[[:space:]]*$"),
    Mark::Shape("^doom: held=[0-9]+ starved=[0-9]+ blocked=[0-9]+ nudged=[0-9]+[[:space:]]*$"),
    Mark::Shape("^sched: kicks=[0-9]+ fallback=[0-9]+[[:space:]]*$"),
    Mark::Shape("^irq: ring=[0-9]+ busy=[0-9]+ idle_ring=[0-9]+ idle_busy=[0-9]+[[:space:]]*$"),
    Mark::Literal("echo: ready"),
    // ── 服务台搬进 SUT 之后（用户裁定"甲 · 只搬纯判据行"）───────────────
    //
    // 一次性那六台（`passer` / `guest` / `sleeper` / `lodger` / `member` / `policy`）的**值判据**
    // 已经搬进它们自己（`cases::Suite`，47 例）⇒ 这里只剩三样，与探针那一刀同一口径：
    //   ① **读数那一行还在**（只查前缀；值归用例）；
    //   ② **登记了几例 / 跑完几例**（基线——少跑一例也拦得住）；
    //   ③ 走通那一句（`exit … note:` ⇒ 正常退场，不是 panic）。
    //
    // **照实记（常驻那四台的"进常驻之前"那一段也搬了）**：`echo` / `uart` / `rtc` / `router`
    // 的读数分开在**几个互不相干的时刻**——`echo` 有 `trip` / `serial` 两趟，三台驱动各有一趟
    // `serve_tree` / `tree_trip`。故**一沓一个 helper、名字取趟名**（见 `crates/cases` 的照实记），
    // 值判据就地判；门这一侧撤掉那四条 tree shape 与 `echo: seq=` / `name miss=`，换成逐沓基线。
    //
    // **照实记（事件循环里那一段仍然没搬，那是另一刀）**：一轮里 `uart: rang` 5 行、
    // `router: exhaust` 6 行——那些在**事件循环里反复发生**，而 `Suite::run()` 是"一趟一次"的
    // 协议 ⇒ 套不上（硬套就得发明"第一次事件时跑一次"或"收官时跑一次"的机制，那是新设计）。
    Mark::Literal("passer: "),
    Mark::Literal("guest: "),
    Mark::Literal("sleeper: "),
    Mark::Literal("lodger: "),
    Mark::Literal("member: "),
    Mark::Literal("policy: "),
    Mark::Shape("^\\[case\\] passer: 1 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] passer: cases 1 ok 1 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] guest: 3 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] guest: cases 3 ok 3 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] sleeper: 3 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] sleeper: cases 3 ok 3 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] lodger: 4 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] lodger: cases 4 ok 4 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] member: 22 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] member: cases 22 ok 22 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] policy: 13 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] policy: cases 13 ok 13 fail 0[[:space:]]*$"),
    // ── 常驻四台"进常驻之前"那一段（用户裁定"甲 · 只搬那一段"）───────────────
    //
    // **一沓一个 helper、名字取趟名**（见 `crates/cases` 的照实记）：`echo` 三沓是**故意的**
    // ——`trip` / `serial` 各自那一刻，而 `seq` 那一格判在 `main`（**返回值只有消耗它的那层
    // 看得见**：`serial` 内部看不见自己那一趟被改坏）。
    Mark::Shape("^\\[case\\] uart-tree: 5 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] uart-tree: cases 5 ok 5 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] rtc-tree: 5 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] rtc-tree: cases 5 ok 5 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] router-tree: 5 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] router-tree: cases 5 ok 5 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] echo-tree: 6 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] echo-tree: cases 6 ok 6 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] echo-serial: 1 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] echo-serial: cases 1 ok 1 fail 0[[:space:]]*$"),
    Mark::Shape("^\\[case\\] echo: 1 cases[[:space:]]*$"),
    Mark::Shape("^\\[case\\] echo: cases 1 ok 1 fail 0[[:space:]]*$"),];
/// 每个前缀的**档**：`auto` = 每一行都得有人判；`narrative` = 没判的行要落在声明的形状里（棘轮，
/// **形状是许可、不是断言**——不要求它出现）；`manual` = 写明为什么不判。
///
/// **照实记（值判据搬进 SUT 之后，形状要跟着放宽）**：判据住在这个文件里时，那四行
/// （`{uart,rtc,router,echo}: tree …`）是 `Mark::Shape` ⇒ 值被钉住。搬进各自那一沓之后，它们
/// 成了**没判的行** ⇒ 必须在这里声明形状，否则 `hold` 报"出格"。**形状按宽松的写**（`part=[0-9]+`、
/// `pname=[^ ]+`）：值由域里那一例判，这里只声明"这一族读数长这样"——写紧了就是把同一条判据
/// 留在两处，红了也说不清是谁判的。
pub const READINGS: &[Reading] = &[
    Reading { prefix: "board", tier: Tier::Auto },
    Reading { prefix: "coalition", tier: Tier::Auto },
    Reading { prefix: "devices", tier: Tier::Auto },
    Reading { prefix: "doom", tier: Tier::Auto },
    Reading { prefix: "guest", tier: Tier::Auto },
    Reading { prefix: "irq", tier: Tier::Auto },
    // **判不了那一格的"为什么"**（`opens gone` / `opens pane` / `opens sealed`）：给读日志的人看的
    // 诊断行，它的**码**由 `probe-rule` 判（`at_pane` / `gone_door` 两例 = `9`），这里判的是
    // "这一族读数还在、且长这样"。写成 `Narrative` 而不是 `Manual`：**不声明**会让任何一行
    // `operator:` 都报"没在表里"，而声明成 `Manual` 又会把那个信号一起放掉——`Narrative`
    // 只许可这一种形状，别的一律出格。
    Reading { prefix: "operator", tier: Tier::Narrative { shapes: &["^operator: opens (gone|pane|sealed) n=[0-9]+$"] } },
    Reading { prefix: "passer", tier: Tier::Auto },
    Reading { prefix: "principal", tier: Tier::Auto },
    Reading { prefix: "probe", tier: Tier::Auto },
    Reading { prefix: "probe-bound", tier: Tier::Auto },
    Reading { prefix: "probe-lease", tier: Tier::Auto },
    Reading { prefix: "probe-other", tier: Tier::Auto },
    Reading { prefix: "probe-owner", tier: Tier::Auto },
    Reading { prefix: "probe-rule", tier: Tier::Auto },
    Reading { prefix: "sched", tier: Tier::Auto },
    Reading { prefix: "sleeper", tier: Tier::Auto },
    Reading { prefix: "timer", tier: Tier::Auto },
    Reading { prefix: "wire", tier: Tier::Auto },
    Reading { prefix: "echo", tier: Tier::Narrative { shapes: &["^(echo: op=[0-9]+|echo: reg=[0-9]+|echo: tree part=[0-9]+ land=[0-9]+ find=[0-9]+ got=(true|false) trim=[0-9]+ plate=[0-9]+ pname=[^ ]+|echo: name miss=(true|false)|echo: seq=[0-9]+)$"] } },
    Reading { prefix: "lodger", tier: Tier::Auto },
    Reading { prefix: "member", tier: Tier::Auto },
    Reading { prefix: "policy", tier: Tier::Auto },
    Reading { prefix: "root", tier: Tier::Narrative { shapes: &["^(root: done)$"] } },
    Reading { prefix: "router", tier: Tier::Narrative { shapes: &["^(router: docks open|router: got [0-9]+|router: tree part=[0-9]+ dir=[0-9]+ land=[0-9]+ find=[0-9]+ got=(true|false) entry=[0-9]+ plate=[0-9]+ pname=[^ ]+)$"] } },
    // **照实记（`rtc: refused` 那一形为什么是 narrative 而不是 `Once`）**：它每轮出现
    // **一到两次**——失败域第一格（过去那个时刻）**每轮必到**，而第二次出现与否正是
    // `sleeper` 那条路上被量出来的那一格（真约那一趟迟到了 ⇒ 也答 `Past`）。条数不声明，
    // 只声明形状：这一行是**读数**，判它的是别处，不是这一门。
    //
    // **照实记（`rtc: asked` 从 `Mark::Once` 搬到这里）**：那一格原先断言"**恰好问一次**"
    // ——那是**老客人的形状**，不是这一面的契约。客人按 `Past` 的文档重问之后（见
    // `harness/src/sleeper.rs::arm_next`），问几次由"那一趟迟没迟"定 ⇒ 契约是**至少一次**。
    // 搬过来仍不放过"一次都没问"：`rtc:` 这一族还有别的 `Once` 判着（`armed` / `rang`）。
    Reading { prefix: "rtc", tier: Tier::Narrative { shapes: &["^(rtc: got [0-9]+|rtc: time [0-9]+ -> [0-9]+|rtc: asked now=[0-9]+|rtc: legs t1=[0-9]+ t2=[0-9]+|rtc: refused=[0-9]+ at=[0-9]+ now=[0-9]+ late_ns=[0-9]+|rtc: tree part=[0-9]+ dir=[0-9]+ land=[0-9]+ find=[0-9]+ got=(true|false) entry=[0-9]+ plate=[0-9]+ pname=[^ ]+)$"] } },
    Reading { prefix: "system", tier: Tier::Narrative { shapes: &["^(system: gone [a-z0-9-]+ state=[A-Za-z]+ ousted=(true|false) heir=[^ ]+ wait=[a-z]+( inner)?)$"] } },
    Reading { prefix: "uart", tier: Tier::Narrative { shapes: &["^(uart: got [0-9]+|uart: tree part=[0-9]+ dir=[0-9]+ land=[0-9]+ find=[0-9]+ got=(true|false) entry=[0-9]+ plate=[0-9]+ pname=[^ ]+)$"] } },
    Reading { prefix: "[case]", tier: Tier::Narrative { shapes: &["^\\[case\\] [a-z0-9-]+: (run|ok) [_a-z0-9]+$"] } },
    Reading { prefix: "task", tier: Tier::Manual { why: "只有停机那一行，由本文件 `verdict`（`HALT` 常量）判——原先是 `soak.sh` 里那段 `if grep -q \"task: all tasks exited, system halted\"`，**那份脚本已删**" } },
];

/// 这一轮读数兑没兑现。**一次报全部缺口**（不是第一条就返回——旧脚本就是"缺这几条"一起报），
/// 但**"这一轮是怎么收的"排在最前**：它是因，后面那些"缺"是果（见 [`stopped`]）。
pub fn verdict(t: &Transcript) -> Result<(), Vec<Gap>> {
    // ① **这一份读数完备吗**：被期限砍断 / 有 panic。照实记（真机红过的那一轮）：这一门原先
    //    只看文本，于是把"45 秒的期限比这一轮短"报成了"读数里没有这一条"。
    let mut missing: Vec<Gap> = stopped(t).into_iter().collect();
    // ② 收场那一句（关机的唯一判据）。没有它就不必往下判——读数还没走到收场；`[stop]` 那几行
    //    是内核的信标（哪一颗核在哪儿空等），"看到什么"就报它。
    if !t.has(HALT) {
        let stop = t
            .text()
            .lines()
            .find(|l| l.contains("[stop]"))
            .unwrap_or("（没有 [stop] 那一行）");
        missing.push(Gap {
            want: "停机行 `task: all tasks exited, system halted`".to_string(),
            saw: stop.to_string(),
        });
        return Err(missing);
    }

    for m in MARKS {
        if !m.holds(t) {
            // `Once` 那一族失败有两种样子（零次 / 两次），故"看到什么"要分开报——不然"出现了两次"
            // 那一格会写着"读数里没有这一条"（自己骗自己）。
            let saw = match m {
                Mark::Once(_) => format!("这一行出现了 {} 次", m.hits(t)),
                _ => "读数里没有这一条".to_string(),
            };
            missing.push(Gap {
                want: format!("这一条读数还在：{}", m.describe()),
                saw,
            });
        }
    }
    // **照实记（这一条判据也搬走了）**：原先这里数三条 `policy: me=`、比"绑 ≠ 领 = 弃"。那三行是
    // `subject` 自己打的，它当然也知道它们该是什么关系 ⇒ 现在是它那一台的一例
    // （`adopting_moves_the_name_and_waiving_puts_it_back`）。宿主这一侧从此不数那三行。
    if let Err(g) = pair(t) {
        missing.push(g);
    }
    // ③ 读数表（[`hold`]）只在上面都没话说时才轮得到——[`stopped`] 报过的那一轮到此为止，
    // 不再逐条列"缺"（列出来只会把因埋进一串回声里）。
    if !missing.is_empty() {
        return Err(missing);
    }

    hold(t, MARKS, READINGS)
}
