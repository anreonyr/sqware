#![no_std]
#![no_main]

//! group — **共享组台**（S 态，**景 `group` 的引导镜像**）：
//! **一次投信，两个等待者都该醒；而那条消息只能归一个人**。
//!
//! # 为什么要有它
//!
//! 「共享组」落地时，它的新行为**在树里没有别的用法**：`root` / `board` 各独享一只
//! **独占**组（`ONLY` ⇒ 任一时刻只有一个能等的持有者 ⇒ 链长恒 ≤ 1），于是"放行全链"与
//! "放行一人"在那两条路径上**是同一件事**——它们证明不了广播。本台子造出**两个真正的
//! 等待者挂在同一只组键上**，把那条路第一次走到，并且是可证伪的：只要"整链放行"退回
//! 成"摘链头一人"，第二个等待者就会睡到永久等 ⇒ 它的回报永远不来 ⇒ 读数当场变红。
//!
//! # 形状
//!
//! ```text
//!   ① 台主造：共享组（Unseal{shared:true}）
//!   ② 成员孔：用户态铸的孔（不带 `ONLY` ⇒ 可复制给两人）
//!   ③ 回报孔**一人一枚**（孔是单槽，共用会撞 `Busy`——那是台子的噪声）
//!   ④ 两个子域各一枚线程，各 accord 一份（组 + 成员 + 自己那枚回报孔）→ hatch
//!   ⑤ 等待者：attach(成员, Pull) → 报 "H" → Await(MAX)（两人挂同一只组键）；台主收齐两份
//!      "H" **才投信**（判据的前提是两人都在等）
//!   ⑥ 对照：独占组在同一位置上的**第二次** accord 必须被拒（源枚已 `HandedOver`）
//!   ⑦ 稳压 200 ms → 成员孔投一字节 → 两个等待者都该被放行
//!   ⑧ 两人各自 `peek`（非破坏性）后回报 "T"；台主自己取那条消息：一次成功、再一次
//!      `Busy` ⇒ "交付只归一人"
//!   ⑨ 收尾：两个等待者都得退场（没醒的那个还在永久等 ⇒ 收掉它）；汇总
//!      `hung=2 woke=2 deliver=true control=true` ⇒ PASS
//! ```
//!
//! # 判据（末行 `group: PASS`；脚本 `crates/gate/tests/group.rs` 只看这一行 + 停机行）
//!
//! - `hung=2`：两人都挂上了（组键上**真的有两个等待者**）；
//! - `woke=2`：**一次投信两人都被放行**——这就是"整链放行"。**退回单播时这里会是 1**
//!   （反向验证做过，见下）；
//! - `deliver=true`：那条消息**只归一个人**（台主取一次成功、再取答 `Busy`）——共享的是
//!   唤醒，不是交付；
//! - `control=true`：**独占组**同样两次 accord 的第二次被拒（共享可复制、独占不可）。
//!
//! # 照实记（三格，都不是内核的事）
//!
//! - **第一版判据不自证伪（反向验证抓出来的）**：原来让醒来的人**直接 `pull`**，用
//!   `took` / `busy` 数"醒了几个人"。把 `knock` 临时改回"摘链头一人"之后台子**照样
//!   PASS**——先到的那个人把槽清空，第二个人醒来只看见空槽，`busy` 被算成了"醒了"。
//!   改成"醒来只 `peek`、消息由台主取走"之后：单播下第二个等待者永远收不到放行、也就
//!   不回报 ⇒ `woke=1` ⇒ FAIL。**判据必须建立在一个不会被它自己吃掉的事实上。**
//! - **对照的 `subset` 必须带 `ONLY`**：第一版写 `FETCH | STORE | VEST` ⇒ **第一次** accord
//!   就被拒——源枚带 `ONLY`、`subset` 不带，`form_ok` 读作"想复制一枚独占资源"。带上
//!   `ONLY` 才是"移交"：第一次成立、第二次因源枚已 `HandedOver` 被拒。**那次拒付本身就是
//!   形态位一致判据的活证据**（`form_ok` 在真机上第一次被触发）。
//! - **`Join` 数不出"干净"**：已经回收干净的任务在名册里没了 ⇒ `Join` 答 `Denied`
//!   （口径是"从未分配 = 非法 id"），故 `reaped` 不能进判据（实测第一版 `reaped=1`，
//!   而两个域都正常退场）。"都退场了"改由机器那一句
//!   `task: all tasks exited, system halted` 担保，脚本读它。
//!
//! # 判据的边界（不是免责）
//!
//! `woke=2` 要证的是"广播"，前提是两人**确已落在 `Await` 上**。握手（③④）把"投信早于
//! 挂格"这一种排除了；剩下的窄窗（报完 "H" 到真的落核之间）由 ⑥ 的 200 ms 稳压盖住——
//! 此刻机器上只有这两枚线程在等，落核不可逆。**即便那个窗口漏掉**，读数也不会假绿成
//! "广播成立"：漏掉时唤醒走的是 `block` 的先探/信标（另一条路），而本条判据问的
//! "放下单播会不会少醒一个人"仍然成立（少醒的那个人永远不报）。
//!
//! # 怎么跑它
//!
//! ```text
//!   crates/gate/tests/group.rs            # 默认 3 轮，判据见上
//!   cargo image group && QEMU_ICOUNT= cargo run --release   # 与验收门同环境（手跑）
//! ```

extern crate alloc;
extern crate programs;

use programs::Reason;

use env::Mark;
use programs::supervisor::root::boot;

use alloc::format;

use env::PieToken;
use env::ProgramKind;
use env::TaskId;
use runtime::core::tole::Tole;
use runtime::env::debug;
use runtime::env::mail::{self, HolePie};
use runtime::env::room;
use runtime::env::unit;

/// 清单里等待者的名字（`env::assembly::ALL` 里 `scenes` 含 `group` 的那一行）。
const WAITER: &str = "waiter";
/// 几名等待者（共享组的重点就是**不止一个**）。
const WAITERS: usize = 2;
/// 回报/收尾的上限（毫秒，**上限族**）。
const MS: usize = 2_000;
/// 投信前的稳压（毫秒；理由见头注的照实记）。
const SETTLE: u64 = 200;

#[programs::entry]
fn main() -> Reason {
    let Some(boot) = boot::Root::take() else { return die("group: boot args unreadable") };
    let Some((elf, kind)) = find(&boot, WAITER) else { return die("group: waiter not in manifest") };

    // ① 组：**共享**（不带 `ONLY` ⇒ 同一枚 accord 给两个任务都成立）。
    let Ok(tole) = Tole::unseal(true) else { return die("group: unseal shared") };
    let group = tole.token();
    // ② 成员：一枚孔（用户态铸的孔不带 `ONLY` ⇒ 也可复制）。
    let Ok(member) = mail::unseal_hole(Mark::of("member")) else { return die("group: member hole") };
    let member = HolePie::from_token(member);
    // ③ 回报孔**一人一枚**：孔是单槽，共用一枚时第二条会撞 `Busy`（那是台子的噪声，
    //    不是被测对象）。
    let mut report = [PieToken::NONE; WAITERS];
    for slot in report.iter_mut() {
        let Ok(tok) = mail::unseal_hole(Mark::of("report")) else { return die("group: report hole") };
        *slot = tok;
    }

    // ④ 两个子域、各一枚线程、各收一份（组 + 成员 + 自己那枚回报孔），放行。
    let mut reps = [TaskId::new(0); WAITERS];
    for i in 0..WAITERS {
        let Ok(team) = unit::build(elf, kind) else { return die("group: build") };
        let Ok(rep) = unit::spawn(team, 0, &[], 0) else { return die("group: spawn") };
        reps[i] = rep;
        // 三枚都按 `FETCH | STORE | VEST` 交出去：够"挂 + 等 + 取 + 回报"这件事本身，
        // 而**两种资源的形态事实都不带 `ONLY`**（共享组与用户态铸的孔）。
        let grant = env::Permission::FETCH | env::Permission::STORE | env::Permission::VEST;
        for tok in [group, member.token(), report[i]] {
            if mail::accord(tok, rep, grant).is_err() {
                return die("group: accord");
            }
        }
        if unit::hatch(rep).is_err() {
            return die("group: hatch");
        }
    }

    // ⑤ 等两份"已挂"：收到才投信（判据的前提——两人都在等）。
    let mut hung = 0usize;
    for i in 0..WAITERS {
        if pull_byte(report[i]) == Some(b'H') {
            hung += 1;
        }
    }

    // ⑥ 对照：**独占组**在同一位置上的第二次 accord 必须被拒（第一次是移交，源枚已
    //    `HandedOver`）。放在这里是因为此刻两个等待者都停在 `Await` 上、表不再变。
    let control = sole_refused(reps[0]);

    // ⑦ 稳压 → 一次投信 → 两个都该醒。
    let _ = room::sleep(core::time::Duration::from_millis(SETTLE));
    let _ = member.push(b"x");

    let mut woke = 0usize;
    for i in 0..WAITERS {
        match pull_byte(report[i]) {
            // "T" = 它 `peek` 看见了那条消息 ⇒ **它被这次投信放行了**。
            Some(b'T') => woke += 1,
            // "E" = 它醒了但复核出了别的错：不算放行 ⇒ 判据自然红。
            _ => {}
        }
    }

    // ⑧ 交付只归一人：**台主自己取**。醒来的人只看不取（`peek`），故槽此刻还是满的——
    //    取一次该成功，再取一次该答 `Busy`。这一格与"整链放行"是两件事：共享的是**唤醒**，
    //    不是**交付**。
    let mut buf = [0u8; 1];
    let deliver =
        member.pull_timeout(&mut buf, 0).is_ok() && member.pull_timeout(&mut buf, 0).is_err();

    // ⑨ 收尾：两个等待者都得退场（**没醒的那个还在永久等** ⇒ 收掉它；这也是"少醒一人"
    //    那一格能被观察到收场的原因）。**这一步不设判据**：「都退场了」由机器那一句
    //    `task: all tasks exited, system halted` 担保（脚本读它），而 `Join` 对已经回收
    //    干净的任务答 `Denied`（名册里没了 = "从未分配"）⇒ 它数不出"干净"这个数。
    for &rep in &reps {
        if !unit::join(rep, MS).unwrap_or(false) {
            let _ = room::doom(rep);
            let _ = unit::join(rep, MS);
        }
    }

    let pass = hung == WAITERS && woke == WAITERS && deliver && control;
    say(&format!(
        "group: hung={hung} woke={woke} deliver={deliver} control={control}"
    ));
    say(if pass { "group: PASS" } else { "group: FAIL" });
    return if pass { 0 } else { 1 };
}

/// 从一枚回报孔取一字节（有界等待；槽空即超时 ⇒ `None`）。
fn pull_byte(tok: PieToken) -> Option<u8> {
    let pie = HolePie::from_token(tok);
    let mut buf = [0u8; 1];
    match pie.pull_timeout(&mut buf, MS) {
        Ok(1) => Some(buf[0]),
        _ => None,
    }
}

/// 对照：**独占组**的两次 accord——第一次移交成功，第二次必须被拒。
///
/// 目标用**已经开始等的那个子域**：它早已认领完自己的三枚（表不再变），多收一枚不带
/// 记号的门闩对它无害；组是台主的弃物，子域退场时那道锚自愈。
///
/// **`subset` 必须带 `ONLY`**：源枚带 `ONLY` 而 subset 不带，正是 `form_ok` 要拒的
/// "想复制一枚独占资源"（第一版就栽在这一格，见头注的照实记）。带上它才是**移交**。
fn sole_refused(dst: TaskId) -> bool {
    let Ok(sole) = Tole::unseal(false) else {
        return false;
    };
    let grant = env::Permission::FETCH
        | env::Permission::STORE
        | env::Permission::VEST
        | env::Permission::ONLY;
    let first = mail::accord(sole.token(), dst, grant);
    let second = mail::accord(sole.token(), dst, grant);
    first.is_ok() && second.is_err()
}

/// 清单里按名字取镜像（只认这一条，与各台主同款）。
fn find(boot: &boot::Root, want: &str) -> Option<(&'static [u8], ProgramKind)> {
    let mut list = boot.programs();
    loop {
        let entry = list.next()?;
        let Ok(entry) = entry else { return None };
        if entry.name == want {
            return Some((entry.elf, entry.kind));
        }
    }
}

/// 打一行读数。台子的嘴只有调试面这一格。
fn say(msg: &str) {
    let _ = debug::put(msg);
}

/// 起不来就报哪一句（内核收场时把这一句连同域号打出来）。
fn die(msg: &str) -> Reason {
    say(msg);
    1
}

