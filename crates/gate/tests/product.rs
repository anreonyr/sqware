//! 产品镜像那一门（**景 `product`**）——**真正要发出去的那一景**（用户裁定"真正要发出去的那一台"）。
//!
//! # 判据（两段：装了什么 · 起不起得来）
//!
//!   1) **装了什么**（宿主侧，读回那颗 `initrd.img`）：**整表**恰好 **6** 条 = 两个域（`root` /
//!      `system`）+ 4 台（3 服务 + `echo`）——**iii 之后持树者 / 身份 / 结盟不再是程序**（它们是
//!      编排域自己域里的三枚线程，不占条数）；且引导镜像落在 `root` 上——`product` 与验收镜像
//!      **共用同一个引导域**，故"景名 = 引导镜像名"那条旧巧合在这里断了
//!      （见 `env::assembly::ENTRY` 的照实记）。
//!   2) **起不起得来**（QEMU）：自己退场 + 停机行 + 六沓用例全绿 + **恰好 4 条 `system: gone`**
//!      （编排域自己的账——起一条记一条。**iii 之后那本账只记"背后有另一个域"的那几台**：本域
//!      那三枚随域退场，不记账，见 `PLAN` 那一格）
//!      + 探针与常客的读数**一条都不在**。
//!
//! # 照实记（这一门为什么不套 `soak` 的读数表）
//!
//! `soak` 吃的是 `MARKS` + `READINGS` 那一整张棘轮表，因为**验收镜像装了表里每一条**。这一景
//! 故意不装那些台 ⇒ 它的读数是**另一张表**。把验收那一份搬过来会变成"拿验收镜像的判据判产品
//! 镜像"——红了也说不清是产品坏了，还是这一景本来就没有那一行。
//!
//! # 照实记（`board:` 那一行只判子串，不锚行尾）
//!
//! 这一门判 `board:` 用的是 [`Mark::Literal`]（子串）——它是**冒烟门**，判的是"这一族读数还在"。
//! **行完整性**那一格不由它看着，由 `soak` 的锚定形状看着（`^board: (bye …|swept …)$`）：
//! 两道门分工不同，不必都锚。
//!
//! **照实记（当初为什么不锚）**：那时量到过 `board: swept n=1 occupied=4system: gone principal …`
//! ——两条读数**胶在一行**上。根因是"一条读数行 = 2~5 次 ecall"，窗口就在两次 ecall 之间；
//! 那一刀已在 `kernel/src/console.rs::_write` 里收掉（**一行攒起来、一次 ecall 发**，照实记在
//! 那里，含实测：12 份现场 3976 行里 1 例）。故 `soak` 那个锚定形状今天**是**一条成立的判据
//! ——它同时也是"这件事有没有复发"的探测器。

use gate::*;
use std::time::Duration;

const HALT: &str = "task: all tasks exited, system halted";

/// 这一景该装的那 **6** 条（**镜像层**：装载次序）。次序是硬事实（`ROOT_OFFSET` 按位次算），
/// 故这里是**整表相等**，不是"含这 6 个"。
///
/// **照实记（iii 之后从 9 条降到 6 条）**：持树者 / 身份 / 结盟不再是**程序**——它们住
/// `prog-system` 那一份字节里（编排域自己的域，`scenario.rs` 的 `INNER`），故不占条数。
const PACKED: &[&str] = &[
    "root", "echo", "router", "uart", "rtc", "system",
];

/// 编排域**记的那 4 台**（**单子层**：按 `order` 排出来的次序）——`echo` 在最后（等它退场才收场）。
///
/// **照实记（iii 之后从 7 台降到 4 台）**：持树者 / 身份 / 结盟不再是被起的程序，而是**编排域
/// 自己域里的三枚线程**（`scenario.rs` 的 `INNER`）。收场那一刀（`stop_running`）**跳过**它们
/// ——收它们就是扑杀本域自己（板线程那一格量过），它们随"域亡＝成员清零"一起走。故它们**不进
/// 这本账**（能记 `Dead` 的，只有"背后有另一个域"的）。
const PLAN: &[&str] = &["router", "uart", "rtc", "echo"];

/// 六沓用例的基线（**逐沓**：登记的例数 = 跑完的例数，且零失败）——少跑一例也拦得住。
const SUITES: &[(&str, usize)] = &[
    ("router-tree", 5),
    ("uart-tree", 5),
    ("rtc-tree", 5),
    ("echo-tree", 6),
    ("echo-serial", 1),
    ("echo", 1),
];

/// 这一景**要有的**读数。**只列这一门当判据的那几条**——不是把验收那一张搬过来。
const MARKS: &[Mark] = &[
    // 引导：内核把设备与那块账交给引导域。
    Mark::Literal("devices: 21 handed to root"),
    Mark::Literal("root: block n=21 region=19 dtb=1 irq=1 bad=0"),
    // 三台设备驱动的配给（类 → 那一段区）：收方要什么、机器给什么，两半在这条线上合拢。
    Mark::Literal("system: router sifive,plic-1.0.0 -> 0xc000000"),
    Mark::Literal("system: uart ns16550a -> 0x10000000"),
    Mark::Literal("system: rtc google,goldfish-rtc -> 0x101000"),
    Mark::Shape("wire: [0-9]+ bytes, paired=true, post=true"),
    // 三台驱动自己：线开了、寄存器配了。
    Mark::Literal("router: docks open"),
    Mark::Literal("router: device_count=95 ctx=1"),
    Mark::Literal("router: line 10 = serial@10000000"),
    Mark::Literal("router: line 11 = rtc@101000"),
    Mark::Literal("uart: ier=rx at=0x10000000"),
    Mark::Literal("uart: line occupied"),
    Mark::Literal("rtc: line occupied"),
    Mark::Literal("rtc: time "),
    // 中断链：设备拉线 ⇒ PLIC ⇒ 内核摇铃 ⇒ 路由者 claim（**回显自己走轮询**，故没有这一条，
    // "链子断了"与"链子好好的"在日志里长得一模一样）。
    Mark::Literal("uart: rang n="),
    Mark::Literal("router: exhaust line=10"),
    // 回显：起来了、拿到控制台、树那面列表也答得出来。
    Mark::Literal("echo: ready"),
    Mark::Literal("echo: console=true"),
    Mark::Literal("echo: list names=sys,device"),
    // 收场：编排域记完账、引导域跟着退（停机行另判 `HALT`）。
    Mark::Literal("system: done"),
    Mark::Literal("root: done"),
    // 板：死一位扫一位（**不锚行尾**，理由见头注）。
    Mark::Literal("board: swept n="),
];

/// 这一景**不该有**的读数：五台探针 + 六位常客（它们住在验收镜像里）。
const ABSENT: &[&str] = &[
    "probe",
    "guest:",
    "passer:",
    "sleeper:",
    "lodger:",
    "member:",
    "subject:",
    // `subject` 那一台自己开的那一沓用例叫 `policy`。
    "policy:",
    // 那两位客人留下的**后果**也一并缺席：房客占的那条线（`lodger`）与客人定的闹钟
    // （`sleeper` 问现在几点、约一个时刻）。
    "lane dropped line=1",
    "rtc: armed at=",
    "rtc: asked now=",
];

/// 读回那颗 `initrd.img` 的清单——**这一景真装了什么**（不是"装配单说装什么"）。
fn packed(image: &Image) -> Vec<String> {
    let at = image.initrd();
    let blob = std::fs::read(&at).unwrap_or_else(|e| panic!("读不到 {}：{e}", at.display()));
    let entries = env::wire::manifest::Entries::new(&blob).expect("清单头非法");
    entries
        .map(|e| e.expect("清单里有一条非法").name.to_string())
        .collect()
}

#[test]
#[ignore = "要起 QEMU"]
fn product() {
    let image = build(Scenario::Product, Profile::Release).expect("造不出那颗要跑的");

    // ── 一、装了什么（宿主侧：读回产物，不读装配单）─────────────────────────
    let got = packed(&image);
    let want: Vec<String> = PACKED.iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want, "产品镜像里装的不对（{} 条）", got.len());

    // ── 二、起不起得来（QEMU）──────────────────────────────────────────────
    // 喂键：等 `echo: ready` 再写 `exit`（它是最后一条，等它退场才收场）。第二条是保险
    // ——第一条往往已经收尾，第二条会撞上断开的管道，那正是预期的收尾（与 `soak` 同一口径）。
    let t = run(&Bench::Machine {
        image: image.clone(),
        sched: Schedule::OnMark {
            mark: "echo: ready",
            within: Duration::from_secs(25),
            then: vec!["exit".to_string(), "exit".to_string()],
        },
        within: secs(45),
        env: &[],
    })
    .expect("机器那一台起不动");
    let _ = t.keep(&log_path("product"));

    let mut gaps: Vec<String> = Vec::new();
    // **先说因**：被期限砍断 / 有 panic（见 `gate::stopped`）——下面那些"不在"多半是它的回声。
    if let Some(g) = stopped(&t) {
        gaps.push(g.to_string());
    }
    if !t.has(HALT) {
        gaps.push(format!("无停机行 `{HALT}`"));
    }
    for m in MARKS {
        if !m.holds(&t) {
            gaps.push(format!("这一条读数不在：{}", m.describe()));
        }
    }
    for m in ABSENT {
        let m = Mark::Absent(m);
        if !m.holds(&t) {
            gaps.push(format!("这一条读数**不该在**：{}", m.describe()));
        }
    }
    // 记的那 4 台：编排域自己的账（起一条记一条）——**整表相等**：多一台少一台都点名。
    match values(&t, "^system: gone ([a-z0-9-]+) state=") {
        Ok(mut names) => {
            names.sort_unstable();
            let mut want = PLAN.to_vec();
            want.sort_unstable();
            if names != want {
                gaps.push(format!(
                    "编排域记的那几台不对：{names:?}（该是 {want:?}）"
                ));
            }
        }
        Err(g) => gaps.push(format!("没有 `system: gone` 那一族的读数：{g}")),
    }
    // 六沓用例：逐沓基线（少跑一例 / 跑一半都拦得住）。
    for (name, n) in SUITES {
        let shape = format!("^\\[case\\] {name}: cases {n} ok {n} fail 0$");
        if let Err(g) = count(&t, &shape, 1) {
            gaps.push(format!("用例基线：{g}"));
        }
    }
    if let Err(g) = pair(&t) {
        gaps.push(format!("用例配对：{g}"));
    }

    assert!(gaps.is_empty(), "产品镜像那一景没过：\n  {}", gaps.join("\n  "));
    println!("product: 6 条镜像 · 4 台在起 · 六沓用例全绿 · 停机行在");
}
