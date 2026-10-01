#![no_std]
#![no_main]

//! group — **共享组台**（S 态，**景 `group` 的引导镜像**）：
//! **一次投信，两个等待者都该醒；而那条消息只能归一个人**。
//!
//! # 为什么要有它
//!
//! 「共享组」落地时，它的新行为**在树里没有别的用法**：那时两个独享者各独享一只
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
//! # 判据（末行 `group: PASS`；嵌入式脚本只看这一行 + 停机行）
//!
//! - `hung=2`：两人都挂上了（组键上**真的有两个等待者**）；
//! - `woke=2`：**一次投信两人都被放行**——这就是"整链放行"。**退回单播时这里会是 1**
//!   （反向验证做过，见下）；
//! - `deliver=true`：那条消息**只归一个人**（台主取一次成功、再取答 `Busy`）——共享的是
//!   唤醒，不是交付；
//! - `control=true`：**独占组**同样两次 accord 的第二次被拒（共享可复制、独占不可）。
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
//!   cargo image group && cargo run        # 手跑
//!   cargo image group && QEMU_ICOUNT= cargo run --release   # 与验收门同环境（手跑）
//! ```

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Reason;

use env::Mark;
use programs::boot::{Accounts, Catalog};

use env::PieToken;
use env::TaskId;
use protocol::debug;
use runtime::core::pile::Pile;
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie, TolePie};
use runtime::env::room;
use runtime::env::unit;

/// 清单里等待者的名字（`programs::unit::PROGRAMS` 里 `wanted_by` 含 `group` 的那一行）。
const WAITER: &str = "waiter";
/// 几名等待者（共享组的重点就是**不止一个**）。
const WAITERS: usize = 2;
/// 回报/收尾的上限（毫秒，**上限族**）。
const MS: usize = 2_000;
/// 投信前的稳压（毫秒；理由见头注）。
const SETTLE: u64 = 200;

#[programs::entry]
fn main() -> Reason {
    let Some(accounts) = Accounts::take() else {
        return die("group: boot args unreadable");
    };
    let Some(waiter) = Catalog::of_boot(&accounts).and_then(|list| list.find(WAITER)) else {
        return die("group: waiter not in manifest");
    };
    let (elf, kind) = (waiter.elf, waiter.kind);

    // ① 组：**共享**（不带 `ONLY` ⇒ 同一枚 accord 给两个任务都成立）。
    let Ok(pile) = Pile::unseal(true) else {
        return die("group: unseal shared");
    };
    let group = pile.token();
    // ② 成员：一枚孔（用户态铸的孔不带 `ONLY` ⇒ 也可复制）。
    let Ok(member) = mail::unseal_hole(Mark::of("member")) else {
        return die("group: member hole");
    };
    let member = HolePie::from_token(member);
    // ③ 回报孔**一人一枚**：孔是单槽，共用一枚时第二条会撞 `Busy`（那是台子的噪声，
    //    不是被测对象）。
    let mut report = [PieToken::NONE; WAITERS];
    for slot in report.iter_mut() {
        let Ok(tok) = mail::unseal_hole(Mark::of("report")) else {
            return die("group: report hole");
        };
        *slot = tok;
    }

    // ④ 两个子域、各一枚线程、各收一份（组 + 成员 + 自己那枚回报孔），放行。
    let mut tasks = [TaskId::new(0); WAITERS];
    for i in 0..WAITERS {
        let Ok(team) = unit::build(elf, kind) else {
            return die("group: build");
        };
        let Ok(task) = unit::spawn(team, 0, &[], 0) else {
            return die("group: spawn");
        };
        tasks[i] = task;
        // 三枚都按 `FETCH | STORE | VEST` 交出去：够"挂 + 等 + 取 + 回报"这件事本身，
        // 而**两种资源的形态事实都不带 `ONLY`**（共享组与用户态铸的孔）。
        let group_pie = TolePie::from_token(group);
        let report_pie = HolePie::from_token(report[i]);
        let form = Policy::VEST;
        if port::ship(&group_pie, task, Access::FETCH_STORE, form).is_err()
            || port::ship(&member, task, Access::FETCH_STORE, form).is_err()
            || port::ship(&report_pie, task, Access::FETCH_STORE, form).is_err()
        {
            return die("group: accord");
        }
        if unit::hatch(task).is_err() {
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
    let control = sole_refused(tasks[0]);

    // ⑦ 稳压 → 一次投信 → 两个都该醒。
    //
    // **（这一手为什么走一次性那一格）**：这一格的读者**只有台主自己**（两位等待者只
    // `peek`，取走是下面第 ⑧ 步台主做的）。孔上那一格要的是一只**递出的手**：`HolePie::push`
    // 只在预算内等到"轮到我"，而"等它被取走"要另写 `wait`——台主此刻正站在这里 ⇒ **自己等自己**。
    // 故这一手写 `Wait::POLL`（一个 envcall，`Ok` = 内核收下了这只手），正好也是这一刀要量的事：
    // **递出即返，交付由取的一方做**。
    let _ = room::sleep(core::time::Duration::from_millis(SETTLE));
    let _ = HolePie::from_token(member.token()).push(b"x", Wait::POLL);

    let mut woke = 0usize;
    for i in 0..WAITERS {
        match pull_byte(report[i]) {
            // "T" = 它 `peek` 看见了那条消息 ⇒ **它被这次投信放行了**。
            Some(b'T') => woke += 1,
            // "E" = 它醒了但复核出了别的错：不算放行 ⇒ 判据自然红。
            _ => {}
        }
    }

    // ⑧ 交付只归一人：**台主自己取**。醒来的人只看不取（`peek`），故那只手此刻还在孔上——
    //    取一次该成功，再取一次该答 `Busy`。这一格与"整链放行"是两件事：共享的是**唤醒**，
    //    不是**交付**。
    let mut buf = [0u8; 1];
    let deliver =
        member.pull(&mut buf, Wait::POLL).is_ok() && member.pull(&mut buf, Wait::POLL).is_err();

    // ⑨ 收尾：两个等待者都得退场（**没醒的那个还在永久等** ⇒ 收掉它；这也是"少醒一人"
    //    那一格能被观察到收场的原因）。**这一步不设判据**：「都退场了」由机器那一句
    //    `task: all tasks exited, system halted` 担保（脚本读它），而 `Join` 对已经回收
    //    干净的任务答 `Denied`（名册里没了 = "从未分配"）⇒ 它数不出"干净"这个数。
    for &task in &tasks {
        if !unit::join(task, Wait::AtMost(MS)).unwrap_or(false) {
            let _ = room::doom(task);
            let _ = unit::join(task, Wait::AtMost(MS));
        }
    }

    let pass = hung == WAITERS && woke == WAITERS && deliver && control;
    debug!("group: hung={hung} woke={woke} deliver={deliver} control={control}");
    debug!("{}", if pass { "group: PASS" } else { "group: FAIL" });
    return if pass { 0 } else { 1 };
}

/// 从一枚回报孔取一字节（有界等待；槽空即超时 ⇒ `None`）。
fn pull_byte(tok: PieToken) -> Option<u8> {
    let pie = HolePie::from_token(tok);
    let mut buf = [0u8; 1];
    match pie.pull(&mut buf, Wait::AtMost(MS)) {
        Ok((1, _)) => Some(buf[0]),
        _ => None,
    }
}

/// 对照：**独占组**的两次 accord——第一次移交成功，第二次必须被拒。
///
/// 目标用**已经开始等的那个子域**：它早已认领完自己的三枚（表不再变），多收一枚不带
/// 记号的门闩对它无害；组是台主的弃物，子域退场时那道锚自愈。
fn sole_refused(dst: TaskId) -> bool {
    let Ok(sole) = Pile::unseal(false) else {
        return false;
    };
    // 同一个子集，只是写成两族：`FETCH | STORE` ＋ `VEST | ONLY`。句柄现造（见 ④ 的注）。
    let form = Policy::VEST | Policy::ONLY;
    let pie = TolePie::from_token(sole.token());
    let first = port::ship(&pie, dst, Access::FETCH_STORE, form);
    let second = port::ship(&pie, dst, Access::FETCH_STORE, form);
    first.is_ok() && second.is_err()
}

/// 起不来就报哪一句（内核收场时把这一句连同域号打出来）。
fn die(msg: &str) -> Reason {
    debug!("{}", msg);
    1
}
