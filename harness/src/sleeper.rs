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
//!   4  arm(now - 1ms)      → sleeper: past=2          失败域第一格（那个时刻已经过去了）
//!   5  arm(now + 50ms)     → sleeper: armed=0         真约；被答 `Past` 就**重问重算**（有界）
//!   6  arm(再约一次)        → sleeper: taken=1         失败域第二格（那一格有人了——就是本域自己）
//!   7  receive()           → sleeper: rang at=<at> now=<t>   等到那一声
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
//! 那台时钟归驱动持有（`ONLY` 是资源事实）。本域**一枚门闩都不要**（装配单里 `needs: None`）
//! ——它只是那一面服务的第一位客人：拿得到的是树上那枚门牌孔的副本，不是设备。
//!
//! # 特权级
//!
//! **U 态**（`plan::assembly::ALL` 里这一行的 `kind`）：铸孔、交孔、上树找服务、一问一答都不需要 S 态。

extern crate alloc;
extern crate programs;

use programs::Report;

// 树：本域是**客侧**（按名找服务）；板：也是客侧（只为让板看见本域的死）。
use protocol::system::operator::call as ocall;
use protocol::system::operator::client as operator;
use protocol::system::board::call as bcall;
use protocol::system::board::client as board;

use alloc::format;
use core::time::Duration;

use env::{Name, PieToken, TaskId};
use protocol::session::Quay;
// 那一面服务：帧形与记号、客侧两手——**与驱动同一份源码**（见 `programs/src/driver/rtc/mod.rs`）。
use programs::driver::rtc::call as rcall;
use programs::driver::rtc::client as clock;
use programs::driver::rtc::core::Fail as RFail;
use cases::Suite;
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::room;
use runtime::env::unit as utask;

/// 本域挂在板上的名字（板按它分人；编排域表里那一条也叫这个）。
const ME: &str = "sleeper";

/// 要找的那位服务在树上的名字：**实时钟**（`/device/rtc`——名字用服务名）。
const WANT: &str = "rtc";

/// 等板 / 等树 / 找一趟服务 / 办一趟往返的总上限（毫秒）。**必须有界**。
const MS: usize = 1000;

/// 找不到就再问一次的间隔（毫秒）：门牌是驱动落的，本域可能比它先起。
const RETRY_MS: usize = 1;

/// 真约的提前量（纳秒）：**它只需要罩住一趟往返**——[`arm_next`] 拿的是**刚问到的**那个
/// `now`（见那一手的照实记）。实测常态一趟 ~12.5 ms（重尾到 0.5 s），这里留约 4× 余量。
const AHEAD_NS: u64 = 50_000_000;

/// 真约那一手最多重问几次（**有界**）：每重问一次就换一个**刚读到的** `now`，故上一次的
/// 迟到不往下累积。
const ARM_TRIES: usize = 5;

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
    let _ = debug::put(&format!("sleeper: reg={reg}"));

    let Ok(sire) = utask::sire() else {
        return no_service("sleeper: no sire");
    };
    let Ok((tree, host)) = operator::open(sire, MS) else {
        return no_service("sleeper: no operator");
    };
    let Ok(talk) = operator::ask_hole(host) else {
        return no_service("sleeper: no talk hole");
    };
    let Some(face) = find_face(&tree, talk, host) else {
        return no_service("sleeper: no rtc plate");
    };
    let _ = debug::put("sleeper: found");

    // 一问一答：现在几点。这一句是后面那两约的**基准**（服务收的是绝对时刻）。
    let Ok(now) = timed("now", || clock::now(face, MS)) else {
        return no_service("sleeper: no time");
    };
    let _ = debug::put(&format!("sleeper: now={now}"));

    // 失败域第一格：一个**已经过去**的时刻。设备对过去的时刻是当场就报，本面选择当场答 `PAST`
    // （见 `programs/src/driver/rtc/core.rs` 那一注）——故这一趟不需要等。
    let past = refused(timed("arm-past", || {
        clock::arm(face, now.saturating_sub(1_000_000), MS)
    }));
    let _ = debug::put(&format!("sleeper: past={past}"));

    // 真约：那一枚回信孔从此留在驱动手里（本域退场之前它一直活着）。
    let Some((armed, at)) = arm_next(face, now) else {
        return no_service("sleeper: no alarm");
    };
    let _ = debug::put(&format!("sleeper: armed={}", rcall::fail_to_code(None)));

    // 失败域第二格：再约一次。那一格里有人——就是本域刚约下的那一次（拿自己的线试，
    // 答 `TAKEN` 是确定的）。
    let taken = refused(clock::arm(face, at.saturating_add(AHEAD_NS), MS));
    let _ = debug::put(&format!("sleeper: taken={taken}"));

    // 等到那一声：**无界等**（本域只有这一件事），而对面一没那枚孔就封印、当场答错。
    let Ok(rang) = armed.receive() else {
        return no_service("sleeper: no ring");
    };
    let _ = debug::put(&format!("sleeper: rang at={at} now={rang}"));

    // 判据就地登记（用户裁定"服务台搬进 SUT"）：**只搬本域已经在判的东西**。三例的期望都是
    // 本站此刻就知道的，而且比较用的是**与读数同一批常量**（`bcall::OK` / `rcall::` 那两个码），
    // 不是新写死的数字。
    //
    // **照实记（`sleeper: armed=0` 那一格没搬）**：它是 `fail_to_code(None)` 打出来的——走到
    // 那一行就恒等于 0，所以"它是 0"是**控制流证据**，不是判据；把它写成
    // `assert_eq!(armed_code, 0)` 就是把 `bail` 改个名字（这一格是写的时候当场撞上的：
    // 第一版写了 `assert!(armed.is_ok())`，而 `armed` 根本不是 `Result`）。
    let mut suite = Suite::new("sleeper");
    suite.case("the_board_took_my_name", move || {
        assert_eq!(reg, bcall::OK)
    });
    suite.case("arming_the_past_is_refused", move || {
        assert_eq!(past, rcall::PAST)
    });
    suite.case("the_slot_is_already_mine", move || {
        assert_eq!(taken, rcall::TAKEN)
    });
    suite.run();

    return Report::note(env::EXIT_OK, "sleeper: gone");
}

/// 真约：拿一个 **`now` 读数**去约；被答 `Past` 就**重问一个 `now`、重算一个 `at`**（有界）。
///
/// **照实记（这一格为什么改过）**：原先是"问一次 `now`、算 `at = now + AHEAD_NS`、约一次"，
/// 而那个 `at` 要**隔两趟往返**才用得上（`now` 那一趟 ＋ 失败域那一趟）。门上量出来的：
/// 常态一趟 **~12.5 ms**、重尾到 **0.5 s**（`rtc: refused=… late_ns=…` 那一行）⇒ 50 ms 那把
/// 尺子随时会输，而输的代价是**整台域死**——`soak` 那一门六条读数（`rtc: armed` /
/// `router: line=11` / `rtc: rang` / `router: exhaust line=11` ＋ 本域两条用例）一起没。
/// `Past` 那一格的文档写着的下一步正是这一句（"重新问一次现在几点、再算一个"）——照它走：
/// **提前量不必猜多大，只要它罩得住一趟**。
///
/// **照实记（那个猜没有被"删掉"，也删不掉）**：`Ask::Arm` 收的是**绝对时刻**，而"这个时刻
/// 过去了没有"只有驱动那一侧的钟说了算 ⇒ 客人**必须**给一个提前量。这一刀改的不是"猜多大"，
/// 是"猜的那一段有多长"（两趟 → 一趟）。真要把猜整个删掉，得让那一问改收**相对量**
/// （"从现在起 x 毫秒"，由驱动在**读到它的那一刻**折算成绝对时刻）——那是帧形与答码的事，
/// 另一刀。
///
/// 返 `(那一枚, 约上的那个时刻)`——后者答话那一行读数要用（`sleeper: rang at=…`）。
fn arm_next(face: PieToken, first: u64) -> Option<(clock::Alarm, u64)> {
    let mut now = first;
    for n in 0..ARM_TRIES {
        let at = now.saturating_add(AHEAD_NS);
        match timed("arm", || clock::arm(face, at, MS)) {
            Ok(alarm) => return Some((alarm, at)),
            // **又晚了** ⇒ 重问一个现在、重算一个 `at`（照实记：这就是 `Past` 那一格写的下一步）。
            Err(RFail::Past) => {
                now = timed("now-retry", || clock::now(face, MS)).ok()?;
                let _ = debug::put(&format!("sleeper: late n={n} now={now}"));
            }
            Err(fail) => {
                // **哪一格失败，落一行**（照实记）：这一格从前只报 `no alarm`，而 `arm` 的三条
                // 失败路——借孔/推帧没走成、答复没来、答了但不是 `OK`——在读数里长得一模一样。
                // 真机上那张"偶尔少一台"的脸就卡在这儿。码本在 `programs/src/driver/rtc/core.rs`：
                // **1 = `Taken`**（那一格有人了）/ **2 = `Past`**（那个时刻已经过去了 ⇒ 上面那一支
                // 已接管）/ **3 = `Denied`**（这一趟自己没走到：孔借不出去 / 帧推不动 / 等到期 /
                // 答话读不懂）。
                let _ = debug::put(&format!(
                    "sleeper: alarm err={}",
                    rcall::fail_to_code(Some(fail))
                ));
                return None;
            }
        }
    }
    None
}

/// 一趟往返掐表：打**两个戳子**（`t0` = 决定要说、`t3` = 答话到手），差由读日志的人算。
///
/// **两个钟同基准**：这一侧与驱动那一侧都读 `chrono::clock()`（"自启动基准的纳秒标量
/// （单调）"），故"出去 / 服务 / 回来"三段可以直接相减；驱动那一侧打的是 `t1`（收到这一帧）
/// 与 `t2`（答话已推出），那一行落在 `t0` 与 `t3` 之间，按值配即可。
///
/// **照实记（量的人不许站进被测的那条路）**：戳子在**调用之前/之后**取，那一行读数在
/// **拿到 `t3` 之后**才打——故它 ~1.08 ms 的价钱落在这一趟之外。
fn timed<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let t0 = runtime::env::chrono::clock().unwrap_or(0);
    let out = f();
    let t3 = runtime::env::chrono::clock().unwrap_or(0);
    let _ = debug::put(&format!("sleeper: legs {what} t0={t0} t3={t3}"));
    out
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
/// 而本域此刻只查了这一趟 ⇒ 这一趟拿走的一定是它（次序见 `programs/src/user/echo.rs` 头注）。
fn find_face(link: &Quay, talk: PieToken, host: TaskId) -> Option<PieToken> {
    let (Ok(dir), Ok(want)) = (Name::new(protocol::driver::DIR), Name::new(WANT)) else {
        return None;
    };
    let road = [dir, want];
    // **间接寻址那一手**：名字先经 `seek` 译成号（"还没挂上"那一格也在这里重试），此后按号。
    let mut left = MS;
    let id = loop {
        match operator::seek(talk, link, &road, MS) {
            Ok(id) => break id,
            Err(ocall::UNKNOWN) if left > 0 => {
                let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
                left = left.saturating_sub(RETRY_MS);
            }
            Err(_) => return None,
        }
    };
    if operator::find(talk, link, id, MS).unwrap_or(ocall::BAD) != ocall::OK {
        return None;
    }
    operator::take(link, host)
}

/// 上板报到（与 `passer` / `echo` 同一段前奏）：返板的答码（`bcall::OK` = 挂上了）。
fn register() -> u8 {
    let Ok(sire) = utask::sire() else {
        return bcall::BAD;
    };
    let Ok((link, board)) = board::open(sire, MS) else {
        return bcall::BAD;
    };
    let Ok(talk) = board::ask_hole(board) else {
        return bcall::BAD;
    };
    let Ok(entry) = mail::unseal_hole(board::ENTRY_MARK) else {
        return bcall::BAD;
    };
    let Ok(me) = Name::new(ME) else {
        return bcall::BAD;
    };
    board::ask(talk, &link, board, bcall::REGISTER, me, entry, MS).unwrap_or(bcall::BAD)
}

