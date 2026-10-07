#![no_std]
#![no_main]

//! 景 group 的引导镜像）：
//! **一次投信，两个等待者都该醒；而那条消息只能归一个人**。
//! # 为什么要有它
//! 「共享组」落地时，它的新行为**在树里没有别的用法**：那时两个独享者各独享一只
//! **独占**组（`ONLY` ⇒ 任一时刻只有一个能等的持有者 ⇒ 链长恒 ≤ 1），于是"放行全链"与
//! "放行一人"在那两条路径上**是同一件事**——它们证明不了广播。本台子造出**两个真正的
//! 等待者挂在同一只组键上**，把那条路第一次走到，并且是可证伪的：只要"整链放行"退回
//! 成"摘链头一人"，第二个等待者就会睡到永久等 ⇒ 它的回报永远不来 ⇒ 读数当场变红。
//! # 形状
//! # 判据（末行 `group: PASS`；嵌入式脚本只看这一行 + 停机行）
//! - `hung=2`：两人都挂上了（组键上**真的有两个等待者**）；
//!   （反向验证做过，见下）；
//! - `deliver=true`：那条消息**只归一个人**（台主取一次成功、再取答 `Busy`）——共享的是
//!   唤醒，不是交付；
//! - `control=true`：**独占组**同样两次 accord 的第二次被拒（共享可复制、独占不可）。
//! # 判据的边界（不是免责）
//! `woke=2` 要证的是"广播"，前提是两人**确已落在 `Await` 上**。握手（③④）把"投信早于
//! 挂格"这一种排除了；剩下的窄窗（报完 "H" 到真的落核之间）由 ⑥ 的 200 ms 稳压盖住——
//! 此刻机器上只有这两枚线程在等，落核不可逆。**即便那个窗口漏掉**，读数也不会假绿成
//! "广播成立"：漏掉时唤醒走的是 `block` 的先探/信标（另一条路），而本条判据问的
//! "放下单播会不会少醒一个人"仍然成立（少醒的那个人永远不报）。
//! # 怎么跑它

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Reason;
use programs::system::loader::{Image, Loader};

use env::Mark;
use programs::boot::{Accounts, Catalog};

use env::pie;
use env::room;
use env::unit;
use env::{PieToken, TaskId};
use protocol::debug;
use ::resource::raw::{Hole};
use ::resource::pile::Pile;
use ::resource::port::{self, Access, Policy};

/// 清单里等待者的名字（programs::unit::PROGRAMS 里 `wanted_by` 含 `group` 的那一行）
const WAITER: &str = "waiter";
/// 几名等待者（共享组的重点就是**不止一个**）
const WAITERS: usize = 2;
/// 回报/收尾的上限（毫秒，**上限族**）
const MS: usize = 2_000;
/// 投信前的稳压（毫秒；理由见头注）
const SETTLE: u64 = 200;

static BUILDERS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

fn concurrent_builders(elf: &'static [u8], kind: env::ProgramKind, authorities: [PieToken; 2]) -> bool {
    use core::sync::atomic::Ordering;
    use execution::unit::task;
    let marks = [Mark::of("group-build"), Mark::of("group-doom")];
    let worker = move || {
        for mark in marks {
            if protocol::communication::session::establish::claim(
                TaskId::new(0), mark, Wait::AtMost(MS),
            ).is_none() {
                return false;
            }
        }
        BUILDERS.fetch_add(1, Ordering::AcqRel);
        let mut spins = 0;
        while BUILDERS.load(Ordering::Acquire) < 2 {
            core::hint::spin_loop();
            spins += 1;
            if spins == 1_000_000 {
                room::starve();
                spins = 0;
            }
        }
        let mut loader = Loader::new();
        let mut children = alloc::vec::Vec::new();
        if children.try_reserve(32).is_err() {
            return false;
        }
        let mut residents = 0;
        for index in 0..32 {
            let Ok(image) = loader.build(Image { bytes: elf, kind }) else {
                return false;
            };
            let team = image.team();
            let Ok(task) = image.spawn(&[], 0) else {
                return false;
            };
            children.push((team, task));
            if index == 0 {
                residents = ::resource::raw::table_size();
            }
        }
        let current = ::resource::raw::table_size();
        if current != residents {
            debug::put(&alloc::format!(
                "group: builder roots {residents}->{current}"
            ));
            return false;
        }
        for (team, task) in children {
            if room::doom(task).is_err() {
                debug::put("group: builder doom");
                return false;
            }
            if unit::join(task, Wait::Forever).is_err() {
                debug::put("group: builder join");
                return false;
            }
            if unit::oust(team).is_err() {
                debug::put("group: builder oust");
                return false;
            }
        }
        true
    };
    let _ = execution::room::park(core::time::Duration::from_millis(100));
    let Ok(left) = task::try_spawn(worker) else {
        return false;
    };
    let _ = execution::room::park(core::time::Duration::from_millis(10));
    let Ok(right) = task::try_spawn(worker) else {
        return false;
    };
    for task in [left.id(), right.id()] {
        for (authority, mark) in authorities.into_iter().zip(marks) {
            if env::pie::accord(authority, task, env::Permission::FETCH, mark).is_err() {
                return false;
            }
        }
    }
    left.join() && right.join()
}

#[programs::entry]
fn main() -> Reason {
    let Some(accounts) = Accounts::take() else {
        return die("group: boot args unreadable");
    };
    let Some(waiter) = Catalog::of_boot(&accounts).and_then(|list| list.find(WAITER)) else {
        return die("group: waiter not in manifest");
    };
    let (elf, kind) = (waiter.elf, waiter.kind);
    let mut loader = Loader::new();
    let Some(authority) = accounts.token(env::Name::Call(env::Call::Build)) else {
        return die("group: build authority missing");
    };
    let Some(doom) = accounts.token(env::Name::Call(env::Call::Doom)) else {
        return die("group: Doom authority missing");
    };
    if !concurrent_builders(elf, kind, [authority, doom]) {
        return die("group: concurrent builders");
    }
    debug::put("group: concurrent builders=64");

    // ① 组：**共享**（不带 `ONLY` ⇒ 同一枚 accord 给两个任务都成立）。
    let Ok(pile) = Pile::unseal(true) else {
        return die("group: unseal shared");
    };
    let group = pile.token();
    // ② 成员：一枚孔（用户态铸的孔不带 `ONLY` ⇒ 也可复制）。
    let Ok(member) = pie::unseal_hole(Mark::of("member")) else {
        return die("group: member hole");
    };
    let member = Hole::from_raw(member);
    // ③ 回报孔**一人一枚**：孔是单槽，共用一枚时第二条会撞 `Busy`（那是台子的噪声，
    //    不是被测对象）。
    let mut report = [PieToken::NONE; WAITERS];
    for slot in report.iter_mut() {
        let Ok(tok) = pie::unseal_hole(Mark::of("report")) else {
            return die("group: report hole");
        };
        *slot = tok;
    }

    // ④ 两个子域、各一枚线程、各收一份（组 + 成员 + 自己那枚回报孔），放行。
    let mut tasks = [TaskId::new(0); WAITERS];
    for i in 0..WAITERS {
        let Ok(image) = loader.build(Image { bytes: elf, kind }) else {
            return die("group: build");
        };
        let Ok(task) = image.spawn(&[], 0) else {
            return die("group: spawn");
        };
        tasks[i] = task;
        // 三枚都按 `FETCH | STORE | VEST` 交出去：够"挂 + 等 + 取 + 回报"这件事本身，
        // 而**两种资源的形态事实都不带 `ONLY`**（共享组与用户态铸的孔）。
        let form = Policy::VEST;
        if port::ship(group, task, Access::FETCH_STORE, form).is_err()
            || port::ship(member.token(), task, Access::FETCH_STORE, form).is_err()
            || port::ship(report[i], task, Access::FETCH_STORE, form).is_err()
        {
            return die("group: accord");
        }
        if unit::embark(task).is_err() {
            return die("group: embark");
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
    let control = sole_refused(tasks[0]);

    // ⑦ 稳压 → 一次投信 → 两个都该醒。
    // `peek`，取走是下面第 ⑧ 步台主做的）。孔上那一格要的是一只**递出的手**：Hole::push
    // **递出即返，交付由取的一方做**。
    let _ = execution::room::park(core::time::Duration::from_millis(SETTLE));
    let _ = Hole::from_raw(member.token()).push(b"x", Wait::POLL);

    let mut woke = 0usize;
    for i in 0..WAITERS {
        match pull_byte(report[i]) {
            Some(b'T') => woke += 1,
            // "E" = 它醒了但复核出了别的错：不算放行 ⇒ 判据自然红。
            _ => {}
        }
    }

    // ⑧ 交付只归一人：**台主自己取**。醒来的人只看不取（`peek`），故那只手此刻还在孔上——
    //    不是**交付**。
    let mut buf = [0u8; 1];
    let deliver =
        member.pull(&mut buf, Wait::POLL).is_ok() && member.pull(&mut buf, Wait::POLL).is_err();

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

/// 从一枚回报孔取一字节（有界等待；槽空即超时 ⇒ `None`）
fn pull_byte(tok: PieToken) -> Option<u8> {
    let pie = Hole::from_raw(tok);
    let mut buf = [0u8; 1];
    match pie.pull(&mut buf, Wait::AtMost(MS)) {
        Ok((1, _)) => Some(buf[0]),
        _ => None,
    }
}

/// 对照：**独占组**的两次 accord——第一次移交成功，第二次必须被拒
/// 目标用**已经开始等的那个子域**：它早已认领完自己的三枚（表不再变），多收一枚不带
fn sole_refused(dst: TaskId) -> bool {
    let Ok(sole) = Pile::unseal(false) else {
        return false;
    };
    // 同一个子集，只是写成两族：`FETCH | STORE` ＋ `VEST | ONLY`。句柄现造（见 ④ 的注）。
    let form = Policy::VEST | Policy::ONLY;
    let pie = sole.token();
    let first = port::ship(pie, dst, Access::FETCH_STORE, form);
    let second = port::ship(pie, dst, Access::FETCH_STORE, form);
    first.is_ok() && second.is_err()
}

/// 起不来就报哪一句（内核收场时把这一句连同域号打出来）
fn die(msg: &str) -> Reason {
    debug!("{}", msg);
    1
}
