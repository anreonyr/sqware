#![no_std]
#![no_main]

//! probe-deep — **深度那一格的证客**：一层层往下 `part`，再一层层剪回来。
//!
//! 树原来有四条**按深度递归**的私有助手（`look` / `holds` / `take` / `put_in`），而一台域的栈
//! 是 `TASK_STACK_SIZE`（16 KiB）。广度有闸（`PANE_CAP = 16`）、一条路有闸（`ROAD_MAX = 8`）、
//! **深度一个闸都没有**——故"往下打"这一手能直接把持树者从栈上打下去。
//!
//! ```text
//!   part(root, "d0") → id0      （老一版：持树者递归 1 层）
//!   part(id0,  "d1") → id1      （递归 2 层）
//!   …
//! ```
//!
//! # 改前的读数（2026-09-24，一次冷启；这一段是这一刀的**负证**）
//!
//! ```text
//!   operator: tid=3                                   ← 持树者（临时一行自报，量完撤）
//!   probe-deep: alive at 32 / 64 / 96
//!   [ERROR] reserved region access: Store at VA(0x1bff8), pc=0x10028
//!   user fault killed: tid=3 cause=15 stval=0x1bff8   ← **持树者自己被杀**
//!   probe-deep: tree tid=21 last=116 cap=512 code=7 after=err:7
//!                                     ↑ 第 117 手答不出   ↑ 连最浅那一手也答不出：命名空间没了
//! ```
//!
//! 三格都量到了：**死的是持树者自己**（`tid=3`，用一行临时自报对上的名字）、**死法不是"答一格
//! 负码"而是整台域被杀**、**后果是命名空间整个消失**。地址也对得上：栈体是
//! `[0x1c000, 0x20000)`、守护页在它下面（`StackWindow::claim` 明说守护页的用处就是"溢出缺页
//! 可诊断"），而 `0x1bff8` = `body_start − 8`——**正好溢出第一帧**；16 KiB ÷ 117 ≈ 每帧 140
//! 字节，与四条递归助手的形状自洽。
//!
//! # 改后的读数（这一刀：号就是表里的下标，四条递归助手一并消失）
//!
//! ```text
//!   probe-deep: tree deep=192 land=0 find=0 clean=1
//!   exit tid=… note: probe-deep: 192 deep, every hand answered
//! ```
//!
//! 同一台探针：**192 层（老死线 117 的 1.6 倍），落一枚、寻回来、从最底剪几层**。
//! 故"深度"从此不是调用栈上的东西——它只决定"你建了多少格"，而那是**容量**问题
//! （与 `PANE_CAP` / `try_reserve` / `Full` 同一条线）。
//!
//! # 它**不进 soak**（照实记：试过四种排法，都不稳）
//!
//! 这一台是**手工量的那一件**：改前（第 117 层死）与改后（192 层活）两边的读数都出自它，
//! 而它在**门里**会咬人——四种排法的实测轨迹：
//!
//! | 排法 | 结果 | 咬在哪 |
//! |---|---|---|
//! | 512 层 + 全剪回来，排在 `echo` 前 | soak **0/10** | 连打 ~1030 手同步往返，把 `echo` 的 1 秒期限挤过期 |
//! | 256 层 + 每 16 手让一手 | soak **8/10** | 让手粒度不够：一次 16 手的突发在慢机上仍吃得掉那 1 秒 |
//! | 256 层 + **每层**让一手 | soak **0/10** | `echo` 全对了，但**探针自己被带走**：`echo` 一退，编排域返回，把还在剪链的它收走（连 `exit` 行都没打出来） |
//! | 挪到装配单**最后** | soak **0/10 "无停机行"** | 编排域等的是最后一条，而它 `board: false`——**板看不见它的死**，那一等没人应 |
//!
//! 故它**装得上电、不上电**（`INITRD_BINS` 里有条目，`PLAN` 里没有）：真机那一对读数是
//! 手工跑的，而**自动的那道门在宿主靶上**——`protocol-case` 的 `operator` 靶的
//! `a_deep_chain_does_not_need_the_call_stack`（把测试线程栈压到 64 KiB 建 500 层链；
//! 退回递归版它当场 `fatal runtime error: stack overflow`，SIGABRT）。那条门**有牙、且不抖**。
//!
//! 这一轮顺带量出三条**装配单的性质**（不管探针去哪，这三条都该记着）：
//!
//! 1. **持树者是串行的**：一台客人连打几百手同步往返，会把别的客人的期限挤过期；
//! 2. **装配单最后一条必须"上板"**（`board: true`）——编排域等的就是它，板看不见它的死就
//!    **永不停机**（把探针放最后那一次的实测症状就是"无停机行"）；
//! 3. **`echo` 一退，编排域返回，会把还在跑的子域一起带走**（探针就是这么被半路带走的）。
//!
//! # 两件照实记
//!
//! 1. **链建在 `/sys/deep` 下**，不落根：落根会动既有的三条读数（`list root=0,3`、
//!    `list names=sys,device`、`list device=4,5,6`），而这一刀**不该动它们**。
//! 2. **每层让一手**（[`YIELD_MS`]）：持树者串行，连打上千手会把别的客人挤到超时——那是这一刀
//!    顺带量出来的**真性质**（记在 `docs/operator-slot.md` §5），而探针不该在门里制造它。
//! 3. **数目字（第几层）不是判据**：帧大小会随代码漂、表也会随实现变。判据是**"改前会死、
//!    改后每一手都答得出"**。**自动的那一半在宿主靶上**——
//!    `operator` 靶::a_deep_chain_does_not_need_the_call_stack`（把测试线程的栈压到
//!    64 KiB 再建 500 层链；退回递归版它 SIGABRT）。
//!
//!    **照实记**：这一句原来写的是"后者在 `scripts/soak.sh` 里"——**不成立**：这一台
//!    **不上电**（`PLAN` 里没有它），soak 里它的断言数是 **0**。它那两行是**手工读数**，
//!    而"手工读数"这件事本身就值得写在这一行上（全库对账的法子见 `scripts/soak.sh` 头注
//!    那一节"这一门断言的是哪几行读数"）。

// 本文件是一份**独立的 bin**（`programs/Cargo.toml` 的 `prog-probe-deep`），**不进 lib**
// ——与 `echo` / `probe-rule` 同一条：`programs/src/user/mod.rs` 里没有它。
//
// 两条 `extern crate` 缺一不可（实测）：`alloc` 是 `format!` 要用；`programs` **不是**为了
// 用它里面的东西，而是为了把 `libprograms` 链进来——**panic handler 与 `_start` 都住那份
// lib**（`programs/src/entry.rs`）。少了它，链接期报 `` `#[panic_handler]` function required ``。
extern crate alloc;
extern crate programs;

use alloc::format;
use alloc::vec::Vec;
use core::time::Duration;

use env::Name;
use protocol::operator::call as ocall;
use protocol::operator::client as operator;
use protocol::operator::{EntryId, Where};
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::room::{self, exit_with_note};
use runtime::env::unit as utask;

/// 一趟的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 往下打多少层。**改前到不了这里**（第 117 层就把持树者打死了），故这一格要的是"比那条死线
/// 高出一截"：192 = 117 的 1.6 倍。**为什么不再深**：这一台每层让一手（见 [`YIELD_MS`]），
/// 而它得在 `echo` 退场之前跑完（编排域等的是 `echo`；`echo` 一退就把还在跑的它带走——实测过
/// 一次：`echo` 那几行全对，而它**连 `exit` 行都没打出来**）。192 层够证那一格，也留得下余地。
/// 链打多深。**公平台上打 512 层**（老表里"512 层 + 全剪回来 ⇒ soak 0/10"那一档：连打
/// ~1030 手同步往返，把 `echo` 的 1 秒期限挤过期）；默认台还是 192。
const MAX_DEPTH: usize = if FAIR { 512 } else { 192 };

/// 剪回来多少层。**不是全剪**：`unlink` 的开销与深度无关（一趟 O(槽数) 的扫），故剪最底这几层
/// 就点得到那一手；而链全剪（192 手往返）会把这一台拖过 `echo` 的命。
/// 从最底剪回几层。公平台上**全剪回来**（凑够那一千来手）；默认台 16。
const TRIM_BACK: usize = if FAIR { 512 } else { 16 };

/// 每几层报一行（"活着"这件事要看得见进度）。
const STEP: usize = 64;

/// **每层让一手**（毫秒）。照实记：持树者是**串行**服务的，一台客人连打几百手会把别的客人挤到
/// 超时（真机实测：不让的那一版 `echo` 的 `part` / `land` / `name` / `trim` / `list` 会连着
/// 1 秒过期——`land=7 plate=0 op=7 seq=1`）。而**"每 16 手让一次"不够**：一次突发（16 手）在
/// 慢机上仍能吃掉那 1 秒。故这里**每层让一手**——持树者在每一个让步的窗口里都排得上别的客人。
///
/// 它不改变这一台要证的任何东西，只把"独占"去掉；而"连打几百手会挤别人"这件事本身是这一刀
/// 顺带量出来的**真性质**，记在 `docs/operator-slot.md` §5 之四（要收它是调度/配额那一族的事）。
const YIELD_MS: u64 = 1;

/// **这一台跑在"公平台"上吗**（`SQWARE_ROOT=fair`，构建期定）。
///
/// 公平台要量的正是"**不让手**会把别人挤成什么样"——故那一台上把让手粒度放回**每 16 手**
/// （就是头注那张表里 soak **8/10** 的那一档：一次 16 手的突发在慢机上仍吃得掉 1 秒）。
/// 默认场景（`root` / `soak`）**一个字不变**：还是每层让一手。
#[cfg(sqware_fair)]
const FAIR: bool = true;
#[cfg(not(sqware_fair))]
const FAIR: bool = false;

/// 这一手要不要让：默认台**每手**都让；**公平台一次都不让**——老表里那一档
/// （"512 层 + 全剪回来，排在 `echo` 前 ⇒ soak 0/10"）就是不让手的版本，而"每 16 手让一次"
/// 那一档实测仍挤不动 `echo`（本台第一次量到：`echo` 五条读数照旧）。
///
/// 让手粒度就是这一台要量的**自变量**：默认台让手（不让门里由探针制造那一格），公平台不让
/// （把那一格做成读数）。
fn should_yield(_n: usize) -> bool {
    !FAIR
}

/// 链的落脚处：`/sys/deep`（**不落根**：root 那三条既有读数一个字都不该动）。
const DIR: &str = "sys";
const PANE: &str = "deep";
/// 最底那一层挂的那一枚的名字。
const LEAF: &str = "leaf";

/// 退场码：全成 / 有一手不成（都不是 panic）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

const OK_NOTE: &str = "probe-deep: 192 deep, every hand answered";
const BAD_NOTE: &str = "probe-deep: a hand stopped answering";

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let Ok(sire) = utask::sire() else {
        bail("probe-deep: no sire")
    };
    let Ok((tree, host)) = operator::open(sire, MS) else {
        bail("probe-deep: no tree link")
    };
    let Ok(talk) = operator::ask_hole(host) else {
        bail("probe-deep: no tree ask")
    };
    let (Ok(dir), Ok(pane), Ok(leaf)) = (Name::new(DIR), Name::new(PANE), Name::new(LEAF)) else {
        bail("probe-deep: bad name")
    };

    // 一、落到 `/sys/deep`（"分"是幂等的，故 `/sys` 已经在也不碍事）。
    let Ok(sys) = operator::part(talk, &tree, Where::Root, dir, MS) else {
        bail("probe-deep: no /sys")
    };
    let Ok(root) = operator::part(talk, &tree, Where::At(sys), pane, MS) else {
        bail("probe-deep: no /sys/deep")
    };
    let mut at = Where::At(root);

    // 二、往下打 [`MAX_DEPTH`] 层。**记下每一层的号**（剪回来要用；树不记父，故这一串是客人
    //     自己的账——这也正是"名字只到 `seek` 那一格，往下一律按号"那句话的形状）。
    let mut chain: Vec<EntryId> = Vec::new();
    let mut code = ocall::OK;
    while chain.len() < MAX_DEPTH {
        let Ok(one) = Name::new(&format!("d{}", chain.len())) else {
            bail("probe-deep: bad name")
        };
        match operator::part(talk, &tree, at, one, MS) {
            Ok(id) => {
                chain.push(id);
                at = Where::At(id);
                if chain.len() % 48 == 0 {
                    say(&format!("probe-deep: alive at {}", chain.len()));
                }
                if should_yield(chain.len()) {
                    let _ = room::sleep(Duration::from_millis(YIELD_MS));
                }
            }
            Err(one) => {
                code = one;
                break;
            }
        }
    }
    let deep = chain.len();

    // 三、**最底那一层真的能用吗**：落一枚砖、寻回来。
    let mut land = ocall::BAD;
    let mut find = ocall::BAD;
    if code == ocall::OK
        && let Ok(entry) = mail::unseal_hole(env::Mark::of("deep-leaf"))
    {
        match operator::land(
            talk,
            &tree,
            host,
            at,
            leaf,
            entry,
            ocall::Rule::Public,
            false,
            MS,
        ) {
            Ok(id) => {
                land = ocall::OK;
                find = operator::find(talk, &tree, id, MS).unwrap_or(ocall::BAD);
                // 那一枚回到本域表里了：认领回来（`take` 取"谁给的"那最后一枚）。
                if let Some(back) = operator::take(&tree, host) {
                    let _ = mail::release(back);
                }
            }
            Err(one) => land = one,
        }
    }

    // 四、**从最底往上剪几层**：`unlink` 那一手（"从父的 children 里摘一号"）也得点到。
    let mut clean = code == ocall::OK;
    for (i, id) in chain.iter().rev().take(TRIM_BACK).enumerate() {
        if operator::trim(talk, &tree, *id, MS).is_err() {
            clean = false;
            break;
        }
        if should_yield(i + 1) {
            let _ = room::sleep(Duration::from_millis(YIELD_MS));
        }
    }

    // 五、一行读数。
    say(&format!(
        "probe-deep: tree deep={deep} land={land} find={find} clean={}",
        u8::from(clean)
    ));

    // 判据：**打满上限**、最底那一层落得上也寻得回、链剪得干净。
    let held =
        deep == MAX_DEPTH && code == ocall::OK && land == ocall::OK && find == ocall::OK && clean;
    exit_with_note(
        if held { E_OK } else { E_TRIP },
        if held { OK_NOTE } else { BAD_NOTE },
    )
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail(note: &str) -> ! {
    say(note);
    exit_with_note(E_TRIP, note)
}

/// 打一行。调试面是本域唯一的嘴（与 `echo` / `guest` 用的是同一格）。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
