//! 默认那一景的读数门（搬自 `scripts/soak.sh`，判据一字未改）。
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

//! # 照实记（`\(` 那一格：一个字都不用改）
//!
//! 搬这批 `needE` 时我先多做了一步——把 `\(` 去成 `(`。**当场红**：`policy: derive(me)=[0-9]+`
//! 在 Rust 的 regex 里是一个**分组**，要的是 `policy: deriveme=12`，而读数是
//! `policy: derive(me)=12`。**grep 的 ERE 与 Rust 的 regex 在这一点上是一致的：`\(` 都是字面
//! 括号** ⇒ 那一格是**恒等变换**。
//!
//! **同一族坑这是第三次**：第一次是声明式对账器把 `need` 的**裸**括号按 ERE 判成分组（22 条
//! 断言永远不命中）；第二次是给它加形状白名单时 `$0 ~ ""` 匹配一切；第三次就是这一次。
//! 括号在这里是雷——**"照抄"比"顺手改"安全**。

use gate::*;
use std::time::Duration;

const HALT: &str = "task: all tasks exited, system halted";

/// 喂键：等探针收尾再喂 `exit`；上限到了**照喂**（那一支注定红在判据上，不是红在超时上）。
const FEED_AFTER: &str = "member: done";

fn feed() -> Schedule {
    Schedule::OnMark {
        mark: FEED_AFTER,
        within: Duration::from_secs(25),
        then: vec!["exit".to_string(), "exit".to_string()],
    }
}

const MARKS: &[Mark] = &[
    Mark::Shape("^router: tree part=0 dir=[0-9]+ land=0 find=0 got=true entry=[0-9]+ plate=[0-9]+ pname=router[[:space:]]*$"),
    Mark::Literal("router: device_count=95 ctx=1"),
    Mark::Literal("uart: ier=rx at=0x10000000"),
    Mark::Literal("system: router sifive,plic-1.0.0 -> 0xc000000"),
    Mark::Literal("system: uart ns16550a -> 0x10000000"),
    Mark::Literal("system: rtc google,goldfish-rtc -> 0x101000"),
    Mark::Literal("system: lodger virtio,mmio -> 0x10001000"),
    Mark::Literal("devices: 21 handed to root"),
    Mark::Shape("^board: (bye tid=[0-9]+ names=[0-9]+ occupied=[0-9]+ swept=[0-9]+|swept n=[0-9]+ occupied=[0-9]+)[[:space:]]*$"),
    Mark::Shape("wire: [0-9]+ bytes, paired=true, post=true"),
    Mark::Shape("passer: reg=0 entry=[0-9]+ say=passer"),
    Mark::Literal("note: passer: gone"),
    Mark::Literal("lodger: gone"),
    Mark::Literal("system: done"),
    Mark::Literal("root: block n=21 region=19 dtb=1 irq=1 bad=0"),
    Mark::Literal("guest: reg=0 find=0"),
    Mark::Literal("guest: trip ok"),
    Mark::Literal("router: line 10 = serial@10000000"),
    Mark::Literal("uart: rang n="),
    Mark::Literal("router: line 11 = rtc@101000"),
    Mark::Literal("rtc: line occupied"),
    Mark::Literal("rtc: armed at="),
    Mark::Shape("^rtc: tree part=0 dir=[0-9]+ land=0 find=0 got=true entry=[0-9]+ plate=[0-9]+ pname=rtc[[:space:]]*$"),
    Mark::Literal("rtc: asked now="),
    Mark::Literal("router: line=11"),
    Mark::Literal("rtc: rang n=1"),
    Mark::Literal("router: exhaust line=11"),
    Mark::Literal("sleeper: reg=0"),
    Mark::Literal("sleeper: found"),
    Mark::Literal("sleeper: now="),
    Mark::Literal("sleeper: past=2"),
    Mark::Literal("sleeper: armed=0"),
    Mark::Literal("sleeper: taken=1"),
    Mark::Literal("sleeper: rang at="),
    Mark::Literal("sleeper: gone"),
    Mark::Literal("router: vacate line=1"),
    Mark::Literal("lodger: taken=2"),
    Mark::Literal("lodger: unknown=1"),
    Mark::Literal("router: lane dropped line=1 pies=21"),
    Mark::Literal("lodger: pies=9"),
    Mark::Literal("router: line 1 = virtio_mmio@10001000"),
    Mark::Literal("router: line=10"),
    Mark::Literal("router: exhaust line=10"),
    Mark::Literal("uart: line occupied"),
    Mark::Literal("lodger: occupy=0"),
    Mark::Shape("^uart: tree part=0 dir=[0-9]+ land=0 find=0 got=true entry=[0-9]+ plate=[0-9]+ pname=uart[[:space:]]*$"),
    Mark::Literal("echo: console=true"),
    Mark::Shape("^echo: tree part=0 land=0 find=0 got=true trim=0 plate=[0-9]+ pname=echo[[:space:]]*$"),
    Mark::Shape("^coalition: tree part=0 dir=[0-9]+ land=0 find=0 got=true entry=[0-9]+ plate=[0-9]+ pname=coalition[[:space:]]*$"),
    Mark::Shape("^principal: tree part=0 dir=[0-9]+ land=0 find=0 got=true entry=[0-9]+ plate=[0-9]+ pname=principal[[:space:]]*$"),
    Mark::Literal("member: found=0"),
    Mark::Literal("member: found=1"),
    Mark::Literal("member: amid(me,c0)=false"),
    Mark::Literal("member: enter(c0)=ok"),
    Mark::Literal("member: leave(c0)=ok"),
    Mark::Literal("member: amid(sub,c0)=true"),
    Mark::Literal("member: amid(sub,c0)=false"),
    Mark::Literal("member: amid(me,c0)=true"),
    Mark::Literal("member: amid(me,out)=err:unknown"),
    Mark::Literal("member: amid(out,me)=false"),
    Mark::Literal("member: enter(c1)=ok"),
    Mark::Literal("member: amid(me,c1)=true"),
    Mark::Literal("member: adopt(sub)=ok"),
    Mark::Literal("member: waive=ok"),
    Mark::Literal("member: enter(out)=err:unknown"),
    Mark::Literal("member: leave(out)=err:unknown"),
    Mark::Literal("member: done"),
    Mark::Literal("policy: sire(root)=none"),
    Mark::Literal("policy: sire(me)=0"),
    Mark::Literal("policy: heir(me,me)=true"),
    Mark::Shape("policy: derive\\(me\\)=[0-9]+"),
    Mark::Literal("policy: heir(sub,me)=false"),
    Mark::Literal("policy: heir(out,me)=err:unknown"),
    Mark::Literal("policy: bind(self)=err:denied"),
    Mark::Literal("policy: adopt(sub)=ok"),
    Mark::Literal("policy: derive(old)=err:denied"),
    Mark::Literal("policy: adopt(up)=err:denied"),
    Mark::Literal("policy: adopt(out)=err:unknown"),
    Mark::Literal("policy: waive=ok"),
    Mark::Literal("subject: done"),
    Mark::Literal("probe: tree land="),
    Mark::Literal("probe-denied: denied as expected"),
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
    Mark::Absent("operator: two asks"),
    Mark::Literal("probe-other: tree is=8 under=8 foreign=8"),
    Mark::Literal("probe-rule-other: all three denied as expected"),
    Mark::Literal("echo: list root=0,3"),
    Mark::Literal("echo: list names=sys,device"),
    Mark::Literal("echo: list device=4,5,6"),
    Mark::Literal("echo: name miss=true"),
    Mark::Literal("echo: seq=0"),
    Mark::Literal("member: band(c0)=n1 more=false"),
    Mark::Literal("member: band(c0,next)=n0 more=false"),
    Mark::Literal("member: band(out)=err:unknown"),
    Mark::Literal("member: bloc(me)=n2 more=false"),
    Mark::Shape("^timer: late_n=[0-9]+ late_max_ms=[0-9]+ late_avg_ms=[0-9]+ late_max_tick=[0-9]+ traps=[0-9]+ tocks=[0-9]+ mutes=[0-9]+[[:space:]]*$"),
    Mark::Shape("^doom: held=[0-9]+ starved=[0-9]+ blocked=[0-9]+ nudged=[0-9]+[[:space:]]*$"),
    Mark::Shape("^sched: kicks=[0-9]+ fallback=[0-9]+[[:space:]]*$"),
    Mark::Shape("^irq: ring=[0-9]+ busy=[0-9]+ idle_ring=[0-9]+ idle_busy=[0-9]+[[:space:]]*$"),
    Mark::Literal("echo: ready"),
];
const READINGS: &[Reading] = &[
    Reading { prefix: "board", tier: Tier::Auto },
    Reading { prefix: "coalition", tier: Tier::Auto },
    Reading { prefix: "devices", tier: Tier::Auto },
    Reading { prefix: "doom", tier: Tier::Auto },
    Reading { prefix: "guest", tier: Tier::Auto },
    Reading { prefix: "irq", tier: Tier::Auto },
    Reading { prefix: "passer", tier: Tier::Auto },
    Reading { prefix: "principal", tier: Tier::Auto },
    Reading { prefix: "probe", tier: Tier::Auto },
    Reading { prefix: "probe-lease", tier: Tier::Auto },
    Reading { prefix: "probe-other", tier: Tier::Auto },
    Reading { prefix: "probe-owner", tier: Tier::Auto },
    Reading { prefix: "probe-rule", tier: Tier::Auto },
    Reading { prefix: "sched", tier: Tier::Auto },
    Reading { prefix: "sleeper", tier: Tier::Auto },
    Reading { prefix: "timer", tier: Tier::Auto },
    Reading { prefix: "wire", tier: Tier::Auto },
    Reading { prefix: "echo", tier: Tier::Narrative { shapes: &["^(echo: op=[0-9]+|echo: reg=[0-9]+)$"] } },
    Reading { prefix: "lodger", tier: Tier::Narrative { shapes: &["^(lodger: got [0-9]+)$"] } },
    Reading { prefix: "member", tier: Tier::Narrative { shapes: &["^(member: derive\\(me\\)=[0-9]+|member: me=[0-9]+)$"] } },
    Reading { prefix: "policy", tier: Tier::Narrative { shapes: &["^(policy: me=[0-9]+)$"] } },
    Reading { prefix: "root", tier: Tier::Narrative { shapes: &["^(root: done)$"] } },
    Reading { prefix: "router", tier: Tier::Narrative { shapes: &["^(router: docks open|router: got [0-9]+)$"] } },
    Reading { prefix: "rtc", tier: Tier::Narrative { shapes: &["^(rtc: got [0-9]+|rtc: time [0-9]+ -> [0-9]+)$"] } },
    Reading { prefix: "system", tier: Tier::Narrative { shapes: &["^(system: gone [a-z0-9-]+ state=[A-Za-z]+ ousted=(true|false) heir=[^ ]+ wait=[a-z]+)$"] } },
    Reading { prefix: "uart", tier: Tier::Narrative { shapes: &["^(uart: got [0-9]+)$"] } },
    Reading { prefix: "[case]", tier: Tier::Narrative { shapes: &["^\\[case\\] [a-z0-9-]+: (run|ok) [_a-z0-9]+$"] } },
    Reading { prefix: "task", tier: Tier::Manual { why: "只有停机那一行：本门开头那句 `t.has(HALT)` 判" } },
    Reading { prefix: "probe-deep", tier: Tier::Manual { why: "**只在公平台起**（默认装配单里没有它）：判据在 `fair` 那一门，不在本门的读数表里" } },
];

#[test]
fn soak() {
    let rounds: usize = std::env::var("GATE_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let image = build(Scenario::Root, Profile::Debug).expect("造不出那颗要跑的");

    let mut pass = 0;
    let mut why_all = Vec::new();
    for i in 1..=rounds {
        let t = run(&Bench::Machine {
            image: image.clone(),
            sched: feed(),
            within: secs(45),
        })
        .expect("机器那一台起不动");
        let _ = t.keep(&log_path(&format!("soak-{i}")));

        if !t.has(HALT) {
            let stop = t
                .text()
                .lines()
                .find(|l| l.contains("[stop]"))
                .unwrap_or("（没有 [stop] 那一行）");
            println!("round {i}: FAIL 无停机行；{stop}");
            why_all.push(format!("round {i}: 无停机行；{stop}"));
            continue;
        }

        let mut missing: Vec<String> = Vec::new();
        for m in MARKS {
            if !m.holds(&t) {
                missing.push(m.describe());
            }
        }
        // 三条 `policy: me=`（装配绑的 / 领之后 / 弃之后）——**不钉号**，钉关系：绑 ≠ 领 = 弃。
        match values(&t, r"^policy: me=([0-9]+)$") {
            Ok(v) if v.len() >= 3 && v[0] != v[1] && v[0] == v[2] => {}
            Ok(v) => missing.push(format!(
                "policy: me= 三条的关系（绑 ≠ 领 = 弃）—— 现在是 {v:?}"
            )),
            Err(g) => missing.push(format!(
                "policy: me= 三条的关系（绑 ≠ 领 = 弃）—— {g}"
            )),
        }
        if let Err(g) = pair(&t) {
            missing.push(g.to_string());
        }

        if !missing.is_empty() {
            println!("round {i}: FAIL 启动读数不全 —— 缺这几条：");
            for m in &missing {
                println!("  {m}");
            }
            why_all.push(format!("round {i}: 启动读数不全（{} 条）", missing.len()));
            continue;
        }

        match hold(&t, MARKS, READINGS) {
            Ok(()) => {
                pass += 1;
                println!("round {i}: PASS");
            }
            Err(gaps) => {
                println!("round {i}: FAIL 读数对账不过 —— 下面这几条是没人管的读数：");
                for g in &gaps {
                    println!("  {g}");
                }
                why_all.push(format!("round {i}: 读数对账不过（{} 条）", gaps.len()));
            }
        }
    }

    assert_eq!(
        pass, rounds,
        "soak {pass}/{rounds}\n  {}",
        why_all.join("\n  ")
    );
}
