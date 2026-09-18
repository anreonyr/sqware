#![no_std]
#![no_main]

//! rig — 压测台：把「杀下去多久收干净」这一格**做成可测的**。
//!
//! 今天这一格只能靠"每个 boot 在收尾那一刀试一次"去猜（~1% 量级），几十轮一批也说明不了
//! 问题。本程序把它换成**每 boot 数百次试验**，而且**时序可控**：
//!
//! ```text
//!   每一轮（同一个受害者 `churn`，反复造/杀）：
//!     造（Build + Spawn，一张新表一行）→ 放行（Hatch）→ 空转 d 轮（扫"点名落在哪一段"）
//!     → 杀（Doom = 域粒度收令）→ 判（`service::until`：now / waited / unsettled）
//!     → 没判出来再看一眼宽限（late / lost）→ 放下那一格（Oust，域干净才放得下）
//! ```
//!
//! 为什么要扫 `d`：那道缝是**时序**的——受害者"在台上"那一段很短，点名落在它的**尾巴**上
//! 才会出现"IPI 打在别处、而它已经挂起"。逐档扫过它的尾巴，才能把这一格从"偶发"变成
//! "哪一档会中"。
//!
//! # 怎么跑它
//!
//! `SQWARE_ROOT=rig cargo run`（`kernel/build.rs` 的 `ROOT_NAME` 读那个环境变量）。
//! 普通 `cargo run` 一个字不变——还是 `root`。`scripts/stress.sh` 是它的壳。
//!
//! # 照实记（装好台子当天量到的）
//!
//! 168 次试验/轮：`rig: total n=168 now=166 waited=2 late=0 lost=0`。也就是说**绝大多数轮里
//! 受害者根本没上过台**——台主（造它的那颗核）不阻塞时，新放行的受害者在**本核**队列里等
//! 着，别的核没把它 `steal` 走（kernel 侧计数：`cull_v=7 nudge=1` ⇒ 8 次里 7 次是"在容器里
//! 被杀"、1 次是"在台上被杀"）。故今天这台子量到的是**快路径**：
//!
//! | 被杀时它在哪 | 结果 |
//! |---|---|
//! | 容器里（队列/挂起） | 100% 同步摘掉（读数 `now`） |
//! | 台上 | 记一笔 + 投递，SSIP 自退（读数 `waited`） |
//!
//! 要量"点名落在它**离核那一瞬**"那一格，上台/离核必须**由台主控制**——**rig A 已经落在
//! 这台子上了**（见下面的 `trial`）：受害者换成 `hang`；它自校准完把孔交回来，台主 `claim`
//! 到它就等于"它已经挂好了"；台主随后 `push` 一记唤醒——于是"醒来跑一小段 → 又挂回去"的
//! 转折点由台主定，杀令的偏移才真的落在"在台上"那一段的时序上。旧版（`churn` + 放行即跑）
//! 的读数留在上面那张表里当历史：那台子量不到这一格。
//!
//! ```text
//!   造(hang) → 台主 seat("wake") → Hatch → hang seat("wake") → 台主 claim 认下它（="它挂好了"）
//!   → 台主 push（正文 = "在台上跑多少轮"，这一句同时就是第一次唤醒）→ 空转 d µs → Doom → 判
//! ```
//!
//! 照实记（**rig A 量到的**，release，`QEMU_SMP=4`；`doom:`/`sched:` 是内核只读读数）：
//!
//! | 跑法 | n | now | waited | late | lost | held/starved/blocked/**nudged** | kicks/steals/tries |
//! |---|---|---|---|---|---|---|---|
//! | 台面 20 ms，扫 d ∈ [0, 20 ms] | 328 | 324 | 4 | 0 | 0 | 0/324/0/**4** | 977/31/443 |
//! | 台面 20 ms，扫 d ∈ [0, 200 ms]（步 5 ms） | 328 | 328 | 0 | 0 | 0 | 0/328/0/**0** | — |
//! | 台面 100 s，d = 1 / 20 / 100 ms（各 32） | 96 | 95 | 1 | 0 | 0 | — | — |
//! | **同上，但 push 之后让出一拍**（`YIELD_AFTER_PUSH`） | 328 | 184 | **142** | 0 | **2** | 0/**0**/184/**146** | 1380/**447**/2279 |
//!
//! 三条结论：
//! 1. **卡点是"源核不 yield"**（甲案落地前最重要的发现）：被唤醒的任务由 `rise` 推进**唤醒那颗核**
//!    （= 台主）的就绪队列，然后旧的无参 `kick()` 只叫醒**一枚**等待核去 `steal`；而 `kick` 的兜底写着
//!    "抢失败则留在源核队列，**源核下次 yield 自取**"——台主是 S 态域任务、空转**不吃陷阱**
//!    ⇒ **永不 yield** ⇒ 任务躺在它的队列里直到被杀（`starved=318/328`）。让出一拍之后：
//!    `starved→0`、`nudged 3→146`、`steals 26→447`，即"唤醒 ⇒ 上台"这一段就通了。
//! 2. **通了之后，"他杀偶发不生效"就复现了**：`lost=2/328`（≈0.6%，与账上那条残差
//!    ~1/320 同量级），另有 `waited=142`（在台上被杀、SSIP 自退）——**rig A 终于量到了它要
//!    量的那一格**。此前各档 `lost=0` 并不是缝不存在，而是台子根本没能把它弄上台。
//! 3. **台子的 `now`/`waited` 分不出"在台上"与"在容器里"**——那是"投递"与"复探"谁先到的赛跑，
//!    不是位置：20 ms 台面下 324/328 是 `now`，而分支计数说真正"在台上被杀"只有 4 次。故本台子
//!    的判据从这一刻起看 `doom: … nudged=…`（内核只读计数），不看 `now/waited` 的分布。
//!
//! # 照实记：这一段的"病"是 **QEMU icount**，不是内核
//!
//! 甲案（`pick` + `kick`：唤醒直投被挑中那颗核）落地后，本台子曾长期量到
//! `doom: … starved=` 312~324/328（"就绪却没上台"），并据此做了一串定位：逐核探针、
//! 门铃确认重试、整字广播、空闲核 1 ms 有界兜底拍。**这些补救全部白费**——真因是
//! **台子与验收门跑在两个环境里**：
//!
//! - `scripts/boot.nu` 的默认是 `-icount auto,sleep=on`：按宿主时间给 vCPU 记账、让它
//!   睡够虚拟额度 ⇒ **WFI 里的核被 IPI 叫醒要等额度**（延迟直方图众数 **1~10 ms**，
//!   尾巴到 10~100 ms）。这解释了当时看到的全部现象：投活 980 笔里 976 笔落在正睡在
//!   WFI 的落点核上、`SendIpi` 一次没返 Err，却只有 393 次 WFI 返回；滞留那 319 笔
//!   "自那笔投活以来那颗核一次都没醒过"。
//! - 而**验收门**（`scripts/examine.nu`）与 `fast.sh` / `probe.sh` 一直是**关着 icount**
//!   跑的。`stress.sh` / `soak.sh` / `load.sh` 没关 ⇒ 两边读数**不可比**。
//!
//! 环境对齐（`scripts/{stress,soak,load}.sh` 显式 `QEMU_ICOUNT=`）之后，同一颗 ELF：
//!
//! | 读数（release，`QEMU_SMP=4`） | icount 开（默认，作废） | **icount 关（与门一致）** |
//! |---|---|---|
//! | `doom: starved` | 312~324 | **0~7 / 328** |
//! | `doom: nudged`（真正"在台上被杀"） | 1~9 | **324~328** |
//! | 落点核自己取走活（`own`） | 24 | **896~1142** |
//! | `sched: steals` | 630 | ~80（**现已随 `steal` 删除，读数一并退休**） |
//! | `rig: total` | now=319~327 waited=1~9 | now=0~6 **waited=322~328** |
//!
//! 5 轮共 1640 次试验的 A/B（`steal` 开 vs 关）：`rig: lost` **1 vs 2**、`doom: starved`
//! **23 vs 18**——两格都在噪声内，故按 task-3 的判据把跨核 `steal` 删掉（判据与实测写在
//! `scheduler::core::fetch` 的文件头）。
//!
//! **注意判据**：本台子的 `now`/`waited` 分不出"在台上"与"在容器里"（那是"投递"与
//! "复探"谁先到的赛跑）——判据看 `doom: … nudged=…`（内核只读计数）与 `rig: total … lost=`。
//!
//! `lost` 照实记：icount 关、环境对齐后 1640 次试验里 `lost=1~2`（≈0.1%，与账上那条
//! 残差 ~1/320 同量级）——**台子现在量到的才是它本来要量的那一格**。

//! 两处要小心：① 每轮台主表里会多留**对端那一枚孔的句柄**（`Quay::shut` 只放本端那一枚）——
//! 一轮一枚，实测 392 轮不敷用的情况没出现，但它是线性增长；② Doom 之后的判决窗口
//! （300 ms + 1 s 宽限）**长于**台面，故 `late`/`lost` 一出现就说明"点名落在尾巴上"真的中过。
//! ③ **甲案落地后 `YIELD_AFTER_PUSH` 默认关** ⇒ `d` 重新是"相对它上台那一刻"的精确偏移
//! （那一拍原本把时序交给了调度器）；要复现甲案前的对照就把开关置 `true`（见上面的读数表）。
//!
//! 顺带量到一条与本题相邻的事实（**已被 `timer::beat_until` 修掉，此处照实留档**）：当时
//! **`Park{millis}` 在"有任务的核"上按拍兑现**（定时器在 trap 里固定重武装 100 ms；只有核进
//! 空闲取活时才按最近的 tock 武装，见 `scheduler/core/fetch.rs`）⇒ 域里 `sleep(1 ms)` 实际是
//! "到下一拍"，`churn` 那个"1 ms 在台上 / 1 ms 离核"的换算因此不成立（它的睡眠段其实是量子级）。
//! 现在四处武装统一为 `min(本核上限, 最近活到点)`，且登记到点的那颗核当场按同一式子重武装 ⇒
//! 该换算成立，本台子的档距假设（下面的 `DELAY_MAX_US`）应当按 `sleep(1 ms)` 的真实时长重取。
//!
//! # 读数
//!
//! 一行一档：`rig: d=<空转轮数> n=.. now=.. waited=.. late=.. lost=..`
//! 末行汇总：`rig: total n=.. now=.. waited=.. late=.. lost=..`
//!
//! `lost` = **300 ms + 宽限 1 s 都没收掉**——那才是"他杀没生效"；`late` = 迟到了但收到。

extern crate alloc;
extern crate programs;

// 共享物住在 supervisor 目录里，由各 bin 各自声明一次（见 `needs.rs` 头注）。
#[path = "../supervisor/needs.rs"]
mod needs;
#[path = "../supervisor/pairing.rs"]
mod pairing;
#[path = "tick.rs"]
mod tick;

use alloc::format;
use core::time::Duration;

use env::Name;
use protocol::session::Quay;
use protocol::system::service::{self, Announce, Reaped, Slot, Table};
use runtime::env::debug;
use runtime::env::room::{self, exit_with};
use runtime::env::unit;

/// 受害者的清单名（`kernel/build.rs::INITRD_BINS`）：**rig A 的握手版受害者**——自校准 →
/// 把孔交给台主 → 无限挂在自己的孔上等人唤醒。旧版 `churn` 仍在清单里（留档），本台子不再用它。
const VICTIM: &str = "hang";

/// 握手那条泊位的名字：**两侧同名**（台主 `seat` 一条、受害者也 `seat` 一条、记号相同才配得齐）。
const LINK: &str = "wake";

/// 等它把手伸出来（`seat`）的上限。它是 `Announce::Channel` 的就绪证据：认领成功 ⇒ 它已经挂好、
/// 可以被唤醒了。
const HANDSHAKE_MS: usize = 1_000;

/// 台主在 push 之后**先让出一拍**再空转（**默认关**）。
///
/// # 照实记：这一拍是甲案落地前的绕行，现在不需要了
///
/// 为什么当初非让不可：被唤醒的任务由 `rise` 推进**唤醒那颗核**（= 台主）的就绪队列，
/// 旧的无参 `kick()` 只叫醒一枚等待核去 `steal`，而它的兜底是"源核下次 yield 自取"——
/// 台主是 S 态域任务、空转不吃陷阱 ⇒ **永不 yield**（见 `conductor::pick` 与
/// `scheduler::core::table::kick` 的照实记）。实测：关掉它 `starved=318/328`、
/// `nudged=3`、`lost=0`；开着它 `starved=0`、`nudged=146`、`lost=2/328`。
///
/// 甲案（`pick` + `kick`）落地后，唤醒**直接落到被挑中那颗核的队列**，不再等源核
/// yield ⇒ 这一拍没有存在的理由：留着它反而把 `d` 扫的时序交给调度器
/// （`d` 不再是"相对它上台那一刻"的精确偏移）。故默认 **false**；置 `true` 可复现
/// 甲案前的那组对照读数（同一台子、同一命令，只差这一拍）。
const YIELD_AFTER_PUSH: bool = false;

/// 本域给它起的服务名（每轮一张**新表**，故名字可以复用）。
const ROW: &str = "victim";

/// 每一档延迟做几轮。
const PER_DELAY: usize = 8;

/// 受害者在台上空转多久（毫秒）：台主把它换算成"多少轮"随第一条消息发过去（见 `hang.rs`）。
///
/// 20 ms 是**扫得动**的台面：档距 500 µs ⇒ 40 档覆盖一整个"在台上"。
const STAGE_MS: usize = 20;

/// 延迟档：**按真实时间扫**（微秒），覆盖受害者"在台上"那一整段（见 `STAGE_MS`）。
const DELAY_MAX_US: usize = 20_000;
const DELAY_STEP_US: usize = 500;

/// 判定窗口（毫秒）：`unsettled` 之后再看宽限（毫秒）——分开"迟到"与"没了"。
const MS: usize = 300;
const LATE_MS: usize = 1_000;

/// 计数（一档一份）。
#[derive(Default, Clone, Copy)]
struct Tally {
    n: usize,
    now: usize,
    waited: usize,
    late: usize,
    lost: usize,
}

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let Some(boot) = pairing::Root::take() else {
        die("rig: boot args unreadable")
    };
    let Some((elf, kind)) = find(&boot, VICTIM) else {
        die("rig: victim not in manifest")
    };
    let Ok(name) = Name::new(ROW) else {
        die("rig: bad row name")
    };

    // 校准：本机"一毫秒 = 多少轮空转"。受害者那边量的是同一把尺。
    let (iters_per_ms, ms_per_tick) = tick::calibrate();
    say(&format!(
        "rig: calib iters_per_ms={iters_per_ms} ms_per_tick={ms_per_tick}"
    ));

    let mut total = Tally::default();
    let mut d_us = 0usize;
    while d_us <= DELAY_MAX_US {
        let mut t = Tally::default();
        for _ in 0..PER_DELAY {
            match trial(name, elf, kind, d_us, iters_per_ms) {
                Ok(verdict) => {
                    t.n += 1;
                    match verdict {
                        Verdict::Now => t.now += 1,
                        Verdict::Waited => t.waited += 1,
                        Verdict::Late => t.late += 1,
                        Verdict::Lost => t.lost += 1,
                    }
                }
                // 造不出来（NoRoom / 表满…）：这一档作罢，照实报出来。
                Err(why) => {
                    say(&format!("rig: d_us={d_us} trial failed: {why}"));
                    break;
                }
            }
        }
        say(&format!(
            "rig: d_us={d_us} n={} now={} waited={} late={} lost={}",
            t.n, t.now, t.waited, t.late, t.lost
        ));
        total.n += t.n;
        total.now += t.now;
        total.waited += t.waited;
        total.late += t.late;
        total.lost += t.lost;
        d_us += DELAY_STEP_US;
    }
    say(&format!(
        "rig: total n={} now={} waited={} late={} lost={}",
        total.n, total.now, total.waited, total.late, total.lost
    ));
    exit_with(0)
}

/// 一轮的判决。
enum Verdict {
    /// 杀令之前就收尾了（同步摘掉）。
    Now,
    /// 问时还没收，**等到收尾事件**后复探确认。
    Waited,
    /// 判定窗口内没收掉，宽限期内收了。
    Late,
    /// 判定窗口 + 宽限都没收掉 —— "他杀没生效"。
    Lost,
}

/// 造一个受害者、放行、空转 `delay` 轮、杀、判、放下。
fn trial(
    name: Name,
    elf: &'static [u8],
    kind: env::ProgramKind,
    delay_us: usize,
    iters_per_ms: usize,
) -> Result<Verdict, &'static str> {
    // 每轮**一张新表**：`Table::register` 一名一行、撤名没有入口，故表本身用完即弃
    // （表是纯值，`Table::new()` 不碰全局）。
    let mut table = Table::new();
    table
        .register(name, Announce::Channel)
        .map_err(|_| "register")?;
    let rep = service::spawn(&mut table, name, elf, kind).map_err(|_| "spawn")?;
    // **rig A：握手**。台主这一侧先 `seat` 一条（顺带给 `claim` 一个"额度"），放行时把码头
    // 交给受害者；它**自校准完**才把自己的孔交回来（`seat`）⇒ 台主 `claim` 到它就等于
    // **"它已经挂好了、可以被唤醒了"**。`start` 丢弃 `ready` 的 bool，故下面显式查 `paired`。
    let Ok(link) = Name::new(LINK) else {
        return Err("bad link name");
    };
    let mut quay = Quay::open(rep);
    quay.seat(link).map_err(|_| "seat")?;
    service::start(
        &mut table,
        name,
        rep,
        &[],
        Some(&mut quay),
        &[link],
        HANDSHAKE_MS,
    )
    .map_err(|_| "start")?;
    let pie = *quay.find(link).ok_or("no pier")?;
    if !pie.paired() {
        return Err("handshake unpaired");
    }
    // ★ 唤醒，并顺手把"在台上跑多少轮"告诉它（**第一句即第一次唤醒**；此后每句都只是唤醒）。
    // 那个轮数由台主**空载校准一次**（`main` 里，铺负荷之前），受害者不自己校准——它每轮都是
    // 一枚新任务，自己校准等于每轮白扔 0.4 s（睡 200 ms + 忙等两格刻度）。
    let burst = iters_per_ms.saturating_mul(STAGE_MS);
    pie.post(&burst.to_le_bytes()).map_err(|_| "post")?;

    // 诊断（默认关）：push 之后**先让出一拍**再空转。判据是 `doom: nudged` 会不会从个位数
    // 跳上去——跳到"几乎每轮"就说明卡点是"源核（台主）空转不 yield"（`kick` 的兜底正是
    // "源核下次 yield 自取"，而 S 态域任务空转不吃陷阱 ⇒ 永不 yield）。
    if YIELD_AFTER_PUSH {
        let _ = room::sleep(Duration::from_millis(1));
    }

    // 扫时序：空转 `delay_us` 微秒再下令（受害者此刻在它"在台上"那一段的某一点上）。
    tick::spin_iters(delay_us.saturating_mul(iters_per_ms) / 1_000);

    // 杀（域粒度收令）+ 判：判决只认非阻塞那一问（见 `service::until`）。
    let _ = service::stop(&mut table, name);
    let verdict = match service::until(&table, name, MS) {
        Ok(Reaped::Now) => Verdict::Now,
        Ok(Reaped::Waited) => Verdict::Waited,
        // 判定窗口内没结论 ⇒ 再看一眼宽限：迟到 vs 没了。
        _ => match service::until(&table, name, LATE_MS) {
            Ok(Reaped::Now) | Ok(Reaped::Waited) => Verdict::Late,
            _ => Verdict::Lost,
        },
    };

    // 放下那一格（域干净才放得下；没收干净就留着——它随本域退场时的级联一起走）。
    if let Some(Slot::Live { team, .. }) = table.find(name).map(|s| s.slot) {
        let _ = unit::oust(team);
    }
    // 本端那一枚孔随码头放下。**对端交上来的那一枚留在本端表里**（`Quay::shut` 只放本端
    // 那一枚）——一轮一枚；够不够用由跑完的读数说话（见头注照实记①）。
    quay.shut();
    Ok(verdict)
}

/// 清单里按名字取镜像（台主只认这一条）。
fn find(boot: &pairing::Root, want: &str) -> Option<(&'static [u8], env::ProgramKind)> {
    let mut list = boot.programs();
    loop {
        let entry = list.next()?;
        let Ok(entry) = entry else { return None };
        if entry.name == want {
            return Some((entry.elf, entry.kind));
        }
    }
}

/// 打一行读数。台主的嘴只有调试面这一格。
fn say(msg: &str) {
    let _ = debug::put(msg);
}

/// 读不出启动账就没得压测。
fn die(msg: &str) -> ! {
    say(msg);
    exit_with(1)
}
