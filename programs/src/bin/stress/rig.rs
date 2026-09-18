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
//! 要量"点名落在它**离核那一瞬**"那一格，上台/离核必须**由台主控制**：受害者无限挂起在
//! 一枚孔上、台主 push 唤醒它（会话那一套 `Quay` 的 seat/claim），于是"醒来跑一小段 → 又挂
//! 回去"的转折点由台主定，杀令的偏移才有意义。
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

use env::Name;
use protocol::system::service::{self, Announce, Reaped, Slot, Table};
use runtime::env::debug;
use runtime::env::room::exit_with;
use runtime::env::unit;

/// 受害者的清单名（`kernel/build.rs::INITRD_BINS`）。
const VICTIM: &str = "churn";

/// 本域给它起的服务名（每轮一张**新表**，故名字可以复用）。
const ROW: &str = "victim";

/// 每一档延迟做几轮。
const PER_DELAY: usize = 8;

/// 延迟档：**按真实时间扫**（微秒），覆盖受害者那一整个来回（在台上 1 ms + 睡 1 ms，
/// 见 `churn.rs`）。档距 ≈ 31 µs；命中哪一档就把那一档附近再扫细。
const DELAY_MAX_US: usize = 2_000;
const DELAY_STEP_US: usize = 31;

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
        .register(name, Announce::None)
        .map_err(|_| "register")?;
    let rep = service::spawn(&mut table, name, elf, kind).map_err(|_| "spawn")?;
    // 放行（门闩空、无会话、不认记号、不等待）：它一起来就进自己那个"空转 + 睡"的循环。
    service::start(&mut table, name, rep, &[], None, &[], 0).map_err(|_| "start")?;

    // 扫时序：空转 `delay_us` 微秒再下令（受害者此刻在它那 1 ms 的某一点上）。
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
