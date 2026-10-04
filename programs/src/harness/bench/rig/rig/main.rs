#![no_std]
#![no_main]

//! 为什么要扫 d：那道缝是时序的——受害者"在台上"那一段很短，点名落在它的尾巴上
//! "哪一档会中"。
//! # 怎么跑它
//! `cargo image rig && cargo run --release`（**场景在造镜像那一刻定**；内核那一份与场景无关，
//! 见 `crates/image`）。**默认那一景现在是 `product`**（验收景 `root` 要写明）。
//! **`--release` 不是偏好，是这一台跑得动的前提**（量于这一轮）：debug 档下
//! `iters_per_ms=3522`、release 是 `24576`（差 7 倍），而"台主空转 `delay_us`"与"受害者上台
//! 20 ms"那两把尺都由这同一个数换算 ⇒ debug 下**每轮都挂在 20 ms 那一缝上**：39 档打完
//! （末行 `rig: d_us=19000 …`）之后**没有汇总行、也没有停机行**，被 `timeout 300` 杀掉。
//! release 下当场绿：
//! ```text
//! total n=328 now=1 waited=327 late=0 lost=0
//! all tasks exited, system halted
//! ```

//! # 台子真正的缺口在**错路**上（读到 → 已补）
//! 查台子用法时读到：`trial()` 里 `register` / `spawn` / `铸那一枚` / `start` / `post` /
//! `handshake unpaired` 那几条早退**都不收场**（不 `oust`、不放本端那一枚）⇒ 真出错时会留下一个
//! 没起或没杀的受害者域、外加本端那枚孔。实测各场**没有一条 `trial failed`**（早退没发生过），
//! 故它一直只是"错路上的账"。
//! **已补，做法是"收场与正文分开"**：一轮一个 `trial`（造 → **不论成没成都收场**）套一段
//! `body`（正文），另把 `LINK` 这条编译期常量提到 `main` 里解一次。于是正文里任何一条早退
//! 都经过同一段收场；剩下不收场的只有 `register` / `spawn`——那时域还没造出来（表是纯值），
//! 没什么可收。**只动台子，不碰内核面。**
//! # 读数
//! 一行一档：`rig: d=<空转轮数> n=.. now=.. waited=.. late=.. lost=..`
//! 末行汇总：`rig: total n=.. now=.. waited=.. late=.. lost=..`

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Reason;
use programs::system::control::core::unit::Declaration;
use programs::system::control::serve::task::{Image, Launch, Readiness};

use env::Mark;
use programs::harness::tick;

use programs::boot::{Accounts, Catalog};

use core::time::Duration;

use alloc::string::{String, ToString};
use programs::system::control::core::unit::{Announce, Slot, Table};
use programs::system::control::core::verdict::Reaped;
use programs::system::control::serve::task as service;
use programs::unit::Ending;
use protocol::communication::session::establish::{self, Endpoint, Held};
use protocol::debug;
use env::unit;
use runtime::core::res::pie::{HolePie};

/// 受害者的清单名（programs::unit::PROGRAMS 里 `wanted_by` 含 `rig` 的那一行）：**rig A 的握手版受害者**——铸一枚孔交给
/// 台主 → 挂在自己那枚孔上等人唤醒。**它不自己校准**：轮数由台主随第一句发过来
const VICTIM: &str = "hang";

/// 握手那条路的记号：**两侧同一个**（台主铸一枚、受害者也铸一枚，刻的都是它才配得齐）
const LINK: &str = "wake";

/// 等它把手伸出来（铸出它那一枚）的上限。它是 Announce::Channel 的就绪证据：认领成功 ⇒ 它已经挂好、
/// 可以被唤醒了
const HANDSHAKE_MS: usize = 1_000;

/// 台主在 push 之后**先让出一拍**再空转（**默认关**）
/// # 这一拍是甲案落地前的绕行，现在不需要了
/// 唤醒**直接落到被挑中那颗核的队列**（`pick` + `kick`），不再等源核 yield ⇒
/// 这一拍没有存在的理由：留着它反而把 `d` 扫的时序交给调度器（`d` 不再是
/// "相对它上台那一刻"的精确偏移）。故默认 **false**；置 `true` 可复现那一拍存在时
/// 的对照读数（同一台子、同一命令，只差这一拍）
const YIELD_AFTER_PUSH: bool = false;

/// 边界细扫开关（**默认关**，见 `main` 里那一段）：把 `d_us` 在 20 ms 附近按 25 µs
/// 细分再扫一遍。开着一轮 656 次试验
/// 留着的理由：`lost`（"他杀不生效"）**只在"点名落在受害者离核那一瞬"那一格出现**
/// 粗扫（500 µs 一档、每档 8 次）抓不到几个样本；细扫之后它变成每轮 0~3 次的可测事件
/// 根因、修法与见 `kernel/.../messenger/doom.rs`
const EDGE_SWEEP: bool = false;

const ROW: &str = "victim";

/// 每一档延迟做几轮
const PER_DELAY: usize = 8;

/// 受害者在台上空转多久（毫秒）：台主把它换算成"多少轮"随第一条消息发过去（见 `hang.rs`）
/// 20 ms 是**扫得动**的台面：档距 500 µs ⇒ 40 档覆盖一整个"在台上"
const STAGE_MS: usize = 20;

/// 延迟档：**按真实时间扫**（微秒），覆盖受害者"在台上"那一整段（见 `STAGE_MS`）
const DELAY_MAX_US: usize = 20_000;
const DELAY_STEP_US: usize = 500;

/// 判定窗口（毫秒）：`unsettled` 之后再看宽限（毫秒）——分开"迟到"与"没了"
const MS: usize = 300;
const LATE_MS: usize = 1_000;

/// 计数（一档一份）
#[derive(Default, Clone, Copy)]
struct Tally {
    n: usize,
    now: usize,
    waited: usize,
    late: usize,
    lost: usize,
}

#[programs::entry]
fn main() -> Reason {
    let Some(accounts) = Accounts::take() else {
        return die("rig: boot args unreadable");
    };
    let Some(victim) = Catalog::of_boot(&accounts).and_then(|list| list.find(VICTIM)) else {
        return die("rig: victim not in manifest");
    };
    let (elf, kind) = (victim.elf, victim.kind);
    let mut loader = programs::system::loader::Loader::new();
    let name = ROW.to_string();
    // 握手那条泊位的名字：**编译期常量**，只解一次——解不出来就不必跑。
    let link = LINK.to_string();

    // 校准：本机"一毫秒 = 多少轮空转"。受害者那边量的是同一把尺。
    let (iters_per_ms, ms_per_tick) = tick::calibrate();
    debug!("rig: calib iters_per_ms={iters_per_ms} ms_per_tick={ms_per_tick}");

    let mut total = Tally::default();
    let mut d_us = 0usize;
    while d_us <= DELAY_MAX_US {
        let mut t = Tally::default();
        for _ in 0..PER_DELAY {
            match trial(&mut loader, name.clone(), link.clone(), elf, kind, d_us, iters_per_ms) {
                Ok(verdict) => {
                    t.n += 1;
                    match verdict {
                        Verdict::Now => t.now += 1,
                        Verdict::Waited => t.waited += 1,
                        Verdict::Late => t.late += 1,
                        Verdict::Lost => t.lost += 1,
                    }
                }
                // 造不出来（`Full`：表满 / 备不下）：这一档作罢，照实报出来。
                Err(why) => {
                    debug!("rig: d_us={d_us} trial failed: {why}");
                    break;
                }
            }
        }
        debug!(
            "rig: d_us={d_us} n={} now={} waited={} late={} lost={}",
            t.n, t.now, t.waited, t.late, t.lost
        );
        total.n += t.n;
        total.now += t.now;
        total.waited += t.waited;
        total.late += t.late;
        total.lost += t.lost;
        d_us += DELAY_STEP_US;
    }

    // 为什么留着它：`他杀偶发不生效`（点名落在受害者**离核那一瞬**）那一格只在
    // `d_us ≈ 20 ms` 出现——粗扫一档 8 轮抓不到几个样本。把 20 ms 附近按 25 µs 细分
    // 之后，它从"每 2~3 轮一次"变成"每轮 0~3 次"（修前 11 次 / 8 轮的读数就是这么
    // 攒出来的；根因与修法见 `kernel/.../messenger/doom.rs` ）。
    // 开着它一轮 656 次试验（粗扫 328 + 细扫 328），故**默认关**。
    let mut b_us = if EDGE_SWEEP { 19_500usize } else { 20_600 };
    while b_us <= 20_500 {
        let mut t = Tally::default();
        for _ in 0..PER_DELAY {
            match trial(&mut loader, name.clone(), link.clone(), elf, kind, b_us, iters_per_ms) {
                Ok(v) => {
                    t.n += 1;
                    match v {
                        Verdict::Now => t.now += 1,
                        Verdict::Waited => t.waited += 1,
                        Verdict::Late => t.late += 1,
                        Verdict::Lost => t.lost += 1,
                    }
                }
                Err(why) => {
                    debug!("rig: edge d_us={b_us} trial failed: {why}");
                    break;
                }
            }
        }
        debug!(
            "rig: edge d_us={b_us} n={} now={} waited={} late={} lost={}",
            t.n, t.now, t.waited, t.late, t.lost
        );
        total.n += t.n;
        total.now += t.now;
        total.waited += t.waited;
        total.late += t.late;
        total.lost += t.lost;
        b_us += 25;
    }
    debug!(
        "rig: total n={} now={} waited={} late={} lost={}",
        total.n, total.now, total.waited, total.late, total.lost
    );
    return 0;
}

/// 一轮的判决
enum Verdict {
    Now,
    /// 问时还没收，**等到收尾事件**后复探确认
    Waited,
    Late,
    Lost,
}

/// 造一个受害者、放行、空转 `delay` 轮、杀、判、放下
/// **收场与正文分开**：`trial` 只管"造 + 收"，一轮的正文在 body。这样造不出来的早退
/// （铸那一枚 / `start` / `no pier` / `handshake unpaired` / `post`）**也照样收场**——否则它们
/// 会留下一个没起或没杀的受害者域，外加本端那枚孔。（这条缺口是查台子用法时读到的
/// 实测各场没有一条 `trial failed`；现在收场不看这一轮成没成。）
/// 只有 `register` / `spawn` 两条仍不收场：那时域还没造出来（表是纯值），没什么可收
fn trial(
    loader: &mut programs::system::loader::Loader,
    name: String,
    link: String,
    elf: &'static [u8],
    kind: env::ProgramKind,
    delay_us: usize,
    iters_per_ms: usize,
) -> Result<Verdict, &'static str> {
    // 每轮**一张新表**：Table::register 一名一行、撤名没有入口，故表本身用完即弃
    // （表是纯值，`Table::new()` 不碰全局）。
    let mut table = Table::new();
    table
        .register(Declaration {
            name: name.clone(),
            announce: Announce::Channel,
            restart: Ending::Transient,
        })
        .map_err(|_| "register")?;
    let task = service::mint(
        &mut table,
        loader,
        Image {
            name: name.as_str(),
            bytes: elf,
            kind,
        },
    )
    .map_err(|_| "spawn")?;
    // **rig A：握手**。台主这一侧先铸一条（`endpoint`：本端那一枚交出去，顺带试认它那一枚），
    // 放行时把通道交给受害者；它铸出自己那一枚交给台主、随即挂在自己那枚孔上 ⇒ 台主 `claim`
    // 到它就等于**"它已经挂好了、可以被唤醒了"**（它**不自己校准**，轮数随后由台主发过去）。
    // `start` 丢弃 `ready` 的 bool，故正文里显式查写端在不在。
    // **有主地建**（`Held(..)`：那一格"有主"由类型说出来）：这一轮的
    // 关系是**真·作用域寿命**（一轮一条、这一轮结束就还回去），
    let held =
        Held(establish::endpoint(task, Mark::of(link.as_str()), Wait::POLL).map_err(|_| "seat")?);
    // `start` 收的是这本账（`&mut [Endpoint]`）；`Endpoint` 是 `Copy` 的号束，故从 `held` 里
    // 取一份出来用，所有权仍在 `held` 手里（放下时放的是同一枚孔）。
    let mut channels = [*held];
    let verdict = body(
        name.clone(),
        task,
        delay_us,
        iters_per_ms,
        &mut table,
        &mut channels,
        link,
    );

    if let Some(Slot::Live {
        team: Some(team), ..
    }) = table.find(name.as_str()).map(|s| s.slot)
    {
        let _ = unit::oust(team);
    }
    // 本端那一枚孔随 `held` 落出作用域放下（`Held` 的 `Drop`）。
    // 对端交上来的那一枚不归我：受害者在 `reap` 里先跑退出钩子（能力级联），台主拿到 `Reaped`
    // 时它已经不在我表里了。
    verdict
}

/// 一轮的正文：起通道之后到判决那一段（**早退也不收场**——收场归 trial）
fn body(
    name: String,
    task: env::TaskId,
    delay_us: usize,
    iters_per_ms: usize,
    table: &mut Table,
    channels: &mut [Endpoint],
    link: String,
) -> Result<Verdict, &'static str> {
    service::embark(
        table,
        Launch {
            task,
            grants: &[],
            readiness: Readiness {
                name: name.as_str(),
                marks: &[Mark::of(link.as_str())],
                wait: Wait::AtMost(HANDSHAKE_MS),
            },
        },
        channels,
    )
    .map_err(|_| "start")?;
    let at_peer = channels.first().and_then(Endpoint::tx).ok_or("no pier")?;
    // ★ 唤醒，并顺手把"在台上跑多少轮"告诉它（**第一句即第一次唤醒**；此后每句都只是唤醒）。
    // 一枚新任务，自己校准等于每轮白扔 0.4 s（睡 200 ms + 忙等两格刻度）。
    // 推的是**对端那一枚**（我写、受害者读），且是**裸字节**（那句轮数不是一族那种报）
    // ⇒ 走裸孔，不套手柄。
    let burst = iters_per_ms.saturating_mul(STAGE_MS);
    // 临时值，不等它下线就返回，受害者会复制到一段死栈。
    let bytes = burst.to_le_bytes();
    let door = HolePie::from_token(at_peer);
    door.push(&bytes, Wait::Forever).map_err(|_| "post")?;
    door.wait(env::HoleDir::Push, Wait::Forever)
        .map_err(|_| "post")?;

    // 诊断（默认关）：push 之后**先让出一拍**再空转。判据是 `doom: nudged` 会不会从个位数
    // 跳上去——跳到"几乎每轮"就说明卡点是"源核（台主）空转不 yield"（`kick` 的兜底正是
    // "源核下次 yield 自取"，而 S 态域任务空转不吃陷阱 ⇒ 永不 yield）。
    if YIELD_AFTER_PUSH {
        let _ = runtime::core::task::sleep(Duration::from_millis(1));
    }

    // 扫时序：空转 `delay_us` 微秒再下令（受害者此刻在它"在台上"那一段的某一点上）。
    tick::spin_iters(delay_us.saturating_mul(iters_per_ms) / 1_000);

    // 杀（域粒度收令）+ 判：判决只认非阻塞那一问（见 service::until）。
    let _ = service::ruin(table, name.as_str());
    Ok(
        match service::until(table, name.as_str(), Wait::AtMost(MS)) {
            Ok(Reaped::Now) => Verdict::Now,
            Ok(Reaped::Waited) => Verdict::Waited,
            // 判定窗口内没结论 ⇒ 再看一眼宽限：迟到 vs 没了。
            _ => match service::until(table, name.as_str(), Wait::AtMost(LATE_MS)) {
                Ok(Reaped::Now) | Ok(Reaped::Waited) => Verdict::Late,
                _ => Verdict::Lost,
            },
        },
    )
}

/// 读不出启动账就没得压测
fn die(msg: &str) -> Reason {
    debug!("{}", msg);
    1
}
