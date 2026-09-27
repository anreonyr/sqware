#![no_std]
#![no_main]

//! sleeper — **客人**：问一声现在几点、约一个时刻、**睡到那一声**、走人。
//!
//! 它是 `rtc` 那台驱动的**真客人**（`/device/rtc` 那块门牌第一位用家）：那台时钟只有持有它的域
//! 读得动（`ONLY`），故"报时 / 定闹钟"这两件事只能由驱动替它做——本域说两句话、收两句话。
//!
//! ```text
//!   1  上板（reg）：只为让板看得见本域的死
//!   2  上树 FIND "/device/rtc"：**找不到就再问**（门牌是驱动落的，本域可能比它先起）
//!   3  now()               → sleeper: now=<t>         一问一答，自带一枚回信孔
//!   4  （**已退场**：那一格是"约一个过去的时刻"，相对量表达不出过去——见下面那条照实记）
//!   5  arm(now + 50ms)     → sleeper: armed=0         真约；被答 `Past` 就**重问重算**（有界）
//!   6  arm(再约一次)        → sleeper: taken=1         失败域第二格（那一格有人了——就是本域自己）
//!   7  receive()           → sleeper: rang after=<ns> now=<t>   等到那一声
//!   8  退场（退场 ⇒ 本域开的那枚孔封印 ⇒ 驱动那一格从此没人收）
//! ```
//!
//! # 失败域那一趟是有意的
//!
//! 与 `lodger` 三趟同一条纪律：**失败域也要卖出读数**，而那两格拿本域自己的线试是**确定**的
//! ——"过去的时刻"由本域自己算得出来（`now` 是刚问的），"那一格有人了"就是本域刚约下的那一次
//! （换别人试会与它抢时间，那是竞态不是读数）。
//!
//! # 为什么它不碰设备
//!
//! 那台时钟归驱动持有（`ONLY` 是资源事实）。本域**一枚门闩都不要**（装配表里 `setup` 是空的）
//! ——它只是那一面服务的第一位客人：拿得到的是树上那枚门牌孔的副本，不是设备。
//!
//! # 特权级
//!
//! **U 态**（`programs::program::PROGRAMS` 里这一行的 `kind`）：铸孔、交孔、上树找服务、一问一答都不需要 S 态。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

// 树：本域是**客侧**（按名找服务）；板：也是客侧（只为让板看见本域的死）。
use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::board as bcall;
use programs::system::board::client as board;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::Face;
use protocol::system::operator::Fail;

use env::{Name, PieToken};
// 那一面服务：帧形与记号、客侧两手——**与驱动同一份源码**（见 `programs/src/driver/rtc/mod.rs`）。
use programs::driver::rtc::client as clock;
use programs::driver::rtc::core::Fail as RFail;
use programs::driver::rtc::core::frame as rcall;
use runtime::env::unit as utask;

/// 本域挂在板上的名字（板按它分人；编排域表里那一条也叫这个）。
const ME: &str = "sleeper";

/// 要找的那位服务在树上的名字：**实时钟**（`/device/rtc`——名字用服务名）。
const WANT: &str = "rtc";

/// 等板 / 等树 / 找一趟服务 / 办一趟往返的总上限（毫秒）。**必须有界**。
const MS: usize = 1000;

/// 这一槽的**周期**（纳秒）："再过这么久叫我"。`Wire::Arm` 收了相对量之后，这个数就是
/// **想要的那段距离本身**，不再是"要罩住一趟往返的提前量"——延迟由收帧的驱动承担
/// （见 `programs/src/driver/rtc/core/frame.rs` 那格的照实记），故它不必再留 4× 余量。
const SLOT_NS: u64 = 50_000_000;

/// 没搭上（找不到那面服务 / 有一条往返没走成）：报这一格退场。
const E_NO_SERVICE: usize = 1;

/// 没搭上：**报码 ＋ 指名是哪一步**。
///
/// **照实记（为什么必须带那句话）**：本域这七条路从前一律 `Report::new(E_NO_SERVICE)`
/// ——**不带 note**。而内核的 `note_out` 对空 note **一行都不打**（分界见 `env::exit` 与
/// `runtime::core::exit`：码表在内核读得到的那一半，note 那一半只归域），于是它**死得
/// 无声**：`soak` 那张"机器好好的、只少了一台"的脸查了很久才落到这里。七处各带一句，
/// 下次它再踩那些 1000 ms 的上限，现场自己说话。
fn no_service(step: &'static str) -> Report<'static> {
    Report::note(E_NO_SERVICE, step)
}

#[programs::entry]
fn main() -> Report<'static> {
    // 上板：**注册在前面**——板要能看见本域（挂不上照样往下走，只是那条信号缺席）。
    let reg = register();
    debug!("sleeper: reg={reg}");

    let sire = utask::sire();
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return no_service("sleeper: no operator");
    };
    // 照实记：从前"树路没接上"与"问话孔没铸出来"是两句 bail —— `Session::open` 把装路那一趟
    // 合成一格，故这里只剩一句。
    //
    // **照实记（这一处为什么包成 `Face`，task-2 那一刀）**：会话装好后本域只要树上那一趟
    // （名字 → 号 → 入口）⇒ 交给 [`Face::of`]（吃所有权），本域那一面叫 `tree`——**避让下面
    // 那个 `face`**（那是 rtc 的服务门牌，另一个东西）。
    let tree = Face::of(session);
    let Some(face) = find_face(&tree) else {
        return no_service("sleeper: no rtc plate");
    };
    debug!("sleeper: found");

    // 一问一答：现在几点。这一句是后面那两约的**基准**（服务收的是绝对时刻）。
    let Ok(now) = clock::now(face, Wait::AtMost(MS)) else {
        return no_service("sleeper: no time");
    };
    debug!("sleeper: now={now}");

    // **照实记（退场的一例：`arming_the_past_is_refused`）**：那一例是"拿 `now - 1ms` 去约，
    // 期望驱动答 `PAST`"。`Wire::Arm` 收了**相对量**之后"过去"**不可表达** ⇒ 判据与它的
    // `sleeper: past=…` 读数一起退场（机制退了，判据也退）。驱动的 `PAST` 那一码留着：`after_ns
    // == 0` 与回绕仍到得了它，只是不再有判据钉着——**这是少了一条判据**，写在这里备查。

    // 真约：那一枚回信孔从此留在驱动手里（本域退场之前它一直活着）。
    let armed = match clock::arm(face, SLOT_NS, Wait::AtMost(MS)) {
        Ok(alarm) => alarm,
        Err(fail) => {
            // **哪一格失败，落一行**（照实记：这一格从前只报 `no alarm`，而 `arm` 的三条失败路
            // ——借孔/推帧没走成、答复没来、答了但不是 `OK`——在读数里长得一模一样）。
            // 码本在 `programs/src/driver/rtc/core/fail.rs`：**1 = `Taken`**（那一格有人了）/
            // **2 = `Past`**（相对量下只剩 `after_ns == 0` 到得了）/ **3 = `Denied`**（这一趟
            // 自己没走到：孔借不出去 / 帧推不动 / 等到期 / 答话读不懂）。
            debug!(
                "sleeper: alarm err={}",
                rcall::fail_to_code(Some(fail))
            );
            return no_service("sleeper: no alarm");
        }
    };
    debug!("sleeper: armed={}", rcall::fail_to_code(None));

    // 失败域第二格：再约一次。那一格里有人——就是本域刚约下的那一次（拿自己的线试，
    // 答 `TAKEN` 是确定的）。
    let taken = refused(clock::arm(face, SLOT_NS, Wait::AtMost(MS)));
    debug!("sleeper: taken={taken}");

    // 等到那一声：**无界等**（本域只有这一件事），而对面一没那枚孔就封印、当场答错。
    let Ok(rang) = armed.receive() else {
        return no_service("sleeper: no ring");
    };
    debug!("sleeper: rang after={SLOT_NS} now={rang}");

    // 判据就地登记（用户裁定"服务台搬进 SUT"）：**只搬本域已经在判的东西**。三例的期望都是
    // 本站此刻就知道的，而且比较用的是**与读数同一批常量**（`bcall::OK` / `rcall::` 那两个码），
    // 不是新写死的数字。
    //
    // **照实记（`sleeper: armed=0` 那一格没搬）**：它是 `fail_to_code(None)` 打出来的——走到
    // 那一行就恒等于 0，所以"它是 0"是**控制流证据**，不是判据；把它写成
    // `assert_eq!(armed_code, 0)` 就是把 `bail` 改个名字（这一格是写的时候当场撞上的：
    // 第一版写了 `assert!(armed.is_ok())`，而 `armed` 根本不是 `Result`）。
    {
        assert_eq!(reg, bcall::OK)
    }
    // 照实记：`arming_the_past_is_refused` 那一例随 `Wire::Arm` 收相对量而退场（"过去"
    // 不可表达）——判据数 3 → 2，`crates/gate/src/soak.rs`（已删）那张表跟着改。
    {
        assert_eq!(taken, rcall::TAKEN)
    }

    return Report::note(env::EXIT_OK, "sleeper: gone");
}

/// 被拒那一趟的读数：把失败域按**线上那张表**折成一个数（与驱动的答码同源）。
fn refused(result: Result<clock::Alarm, RFail>) -> u8 {
    match result {
        Ok(_) => rcall::fail_to_code(None),
        Err(fail) => rcall::fail_to_code(Some(fail)),
    }
}

/// 找那面服务：`FIND /device/rtc`，**找不到就再问**（有界）——门牌是驱动落的，本域可能比它先起。
///
/// 找到之后那一枚**从会话里**进本域表（报文里没有号）：认的是"持树者刚授进来的那一份"，
/// 而本域此刻只查了这一趟 ⇒ 这一趟拿走的一定是它。
///
/// **照实记（收 `&Face`，不再收 `&Session`）**：调用方**已持**一面（task-2 那一刀把它包出来了），
/// 故这一手只借它——签名上不再出现那条线。
fn find_face(tree: &Face) -> Option<PieToken> {
    let (Ok(dir), Ok(want)) = (Name::new(protocol::driver::DIR), Name::new(WANT)) else {
        return None;
    };
    // 名字 → 号（**译不出就重试**：门牌是驱动落的，它可能落得比本域晚）→ 入口：两格在
    // [`Pane::tile`] 与 [`Tile::token`] 上（旧 `Face::tile` 那一趟；本域从前自己抄了一遍）。
    //
    // **照实记（task-2 那一刀；为什么不用 `Face::tile`）**：`entry` 自己已经译号一次 + `find`
    // 一次，随后 `Tile::token` 又 `find` 一次 ⇒ 每趟多授一枚没人接的副本进本域表。旧面只有
    // 一枚，故这里也照一枚写（重试那一圈照旧留着）。
    let root = tree.root();
    let road = [dir, want];
    let mut left = MS;
    loop {
        match root
            .tile(&road, Wait::AtMost(MS))
            .and_then(|entry| entry.token(Wait::AtMost(MS)))
        {
            Ok(entry) => return Some(entry),
            Err(Fail::Unknown) if left > 0 => {
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
                left = left.saturating_sub(1);
            }
            Err(_) => return None,
        }
    }
}

/// 上板报到（与 `passer` / `canonical` 同一段前奏）：返板的答码（`bcall::OK` = 挂上了）。
fn register() -> u8 {
    let sire = utask::sire();
    let Ok(seat) = Session::open(sire, board::BERTH, Wait::AtMost(MS)) else {
        return bcall::BAD;
    };
    // 照实记：从前"板路没接上"与"问话孔没铸出来"是两句 bail（这里折成同一个 `BAD`）——
    // `Session::open` 把装路那一趟合成一格。解入口、编名字、`register` 三手由 `enroll` 收成一手。
    board::enroll(&seat, ME, Wait::AtMost(MS)).0
}
