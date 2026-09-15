#![no_std]
#![no_main]

//! root — 装配者：产生域、分发门闩，并在用户会话结束后收场。
//!
//! ```text
//! 1  启动参数 → 清单（有哪些程序）与配对块（有哪些门闩）——两块都是 boot 只读借映的
//! 2  建 plic 域（Held）→ 按**需求单**取门闩交出去 → Hatch → 收它的控制孔 → 推记录
//! 3  建 echo（U 态）→ Spawn → Hatch
//! 4  等 echo 退场 → 本域退出 ⇒ doom 级联 ⇒ 全部回收 ⇒ 自然停机（srst）
//! ```
//!
//! **退出即关机**，故门的两条硬判据（自行退出 + 无 panic）照旧成立，不需要外接 timeout。
//! plic 域是常驻的（它没有"干完"这回事），由本域退场时的级联收掉。
//!
//! # 本域是设备门闩的**第一个持有者**，也是转授者
//!
//! boot 把设备树扫成门闩、连名字一起写进配对块，整块借映进本域（`platform/devices.rs`）。
//! 本域**不解释设备语义**——它只按名字挑出要交出去的那几枚。**要什么由要的人开单子**，
//! 而那张单子是**两端共用的一份**（`programs::needs`）：本域不手抄名字，也不猜权与形态。
//!
//! 清单与启动参数的**字节布局**同样不在这里定义：前者在 `env::wire::manifest`（与打包它
//! 的内核 `build.rs` 同一份），后者在 `env::wire::args`。跨域的字节格式不留第二份账。
//!
//! # 为什么要有"收控制孔"那一步
//!
//! `Accord` 只解决"父 → 子"这一向：本域拿到的是**子域表里**的句柄。反方向没有解——
//! 子域对自己表里有什么一无所知（`collect` 报句柄、权限、授与人，**不报类型与名字**）。
//! 故子域自铸一条孔授回来，本域按 `vestor == 子域` 在自己表里认出它，再把
//! 「名字 + 句柄」的记录推过去——**名字跟着门闩走**，不靠位置约定。
//!
//! 记录用 `env::Pair`：**内核交给本域的就是这个格式**（配对块），不新造一种。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

use alloc::vec::Vec;

use env::wire::{args as boot_args, manifest};
use env::{Name, PAIR_LEN, Pair, PieToken, TaskId};
use programs::needs::{self, Kind};
use runtime::core::port::ship;
use runtime::env::debug;
use runtime::env::mail::{self, HolePie, NolePie, PolePie};
use runtime::env::room::{exit_with, starve};
use runtime::env::unit as utask;

/// 本域产生的两个子域（清单里的名字）。
const PLIC: &str = "plic";
const ECHO: &str = "echo";

/// 等子域控制孔的上限（轮次）。**必须有界**：子域要是死在头几步，本域不能陪着挂死。
const QUAY_TRIES: usize = 200_000;

/// 失败编号：指"死在装配的哪一步"（沿用旧树那套小整数编号的意思）。
const E_ARGS: usize = 1;
const E_MANIFEST: usize = 2;
const E_PROGRAM: usize = 3;
const E_WIRE: usize = 4;
const E_BUILD: usize = 5;
const E_SPAWN: usize = 6;
const E_HATCH: usize = 7;
const E_EXIT: usize = 0;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 启动参数（布局见 `env::wire::args`）：清单区 VA/字节数、配对块 VA/条数；两块都只读。
    let a = utask::args();
    if a.len() < boot_args::LEN {
        exit_with(E_ARGS);
    }
    let (view, len) = (a[boot_args::VIEW] as *const u8, a[boot_args::VIEW_LEN]);
    let (pairs, count) = (a[boot_args::PAIRS] as *const u8, a[boot_args::COUNT]);

    // 清单（格式见 `env::wire::manifest`）：本域只挑自己要起的那两个，别的一概不看。
    // SAFETY: boot 把这一区只读映射进本域，长度即 region 的字节数；本域只读。
    let blob = unsafe { core::slice::from_raw_parts(view, len) };
    let Some(list) = manifest::Entries::new(blob) else {
        exit_with(E_MANIFEST);
    };
    let (mut plic, mut echo) = (None, None);
    for item in list {
        let Ok(entry) = item else {
            exit_with(E_MANIFEST);
        };
        if entry.name == PLIC {
            plic = Some(entry);
        } else if entry.name == ECHO {
            echo = Some(entry);
        }
    }
    let (Some(plic), Some(echo)) = (plic, echo) else {
        exit_with(E_PROGRAM);
    };

    // 2. 中断面域：建出来、按需求单配给、放行、把名字递过去。
    if !wire_plic(&plic, pairs, count) {
        say("root: wire plic failed");
        exit_with(E_WIRE);
    }

    // 3. 用户会话：echo 是 U 态，不要服务、不要设备，故什么都不用配。
    let Ok(team) = utask::build(echo.elf, echo.kind, echo.name) else {
        exit_with(E_BUILD);
    };
    let Ok(child) = utask::spawn(team, 0, &[], 0) else {
        exit_with(E_SPAWN);
    };
    if utask::hatch(child).is_err() {
        exit_with(E_HATCH);
    }

    // 4. 等它走。本域退出 ⇒ 级联扑杀（plic 也在内）⇒ 全部任务回收 ⇒ 停机。
    join_done(child);
    exit_with(E_EXIT)
}

/// 建中断面域，按它的**需求单**（`programs::needs::PLIC`）把门闩一枚一枚交出去。
/// 返 false = 哪一步没成。
///
/// 顺序是契约：**先 Accord 再 Hatch**——子域放行时它要的已经在它表里，它不必等。
fn wire_plic(entry: &manifest::Entry, pairs: *const u8, count: usize) -> bool {
    // Held：还没放行，故 Accord 落在它表里时它一步都还没跑。
    let Ok(team) = utask::build(entry.elf, entry.kind, entry.name) else {
        return false;
    };
    let Ok(child) = utask::spawn(team, 0, &[], 0) else {
        return false;
    };

    // 一条需求走一遍：按名字从配对块取门闩 → 按它要的权与形态授出 → 记下种在子域表里的
    // 句柄。**种类决定挑哪一层句柄**——那是需求单里唯一"壳"的信息，设备语义一概不在。
    let mut records: Vec<Pair> = Vec::with_capacity(needs::PLIC.len());
    for need in needs::PLIC {
        let Some(token) = pair_token(pairs, count, need.name) else {
            return false;
        };
        let at = match need.kind {
            Kind::Pole => ship(&PolePie::from_token(token), child, need.access, need.policy),
            Kind::Nole => ship(&NolePie::from_token(token), child, need.access, need.policy),
        };
        let Ok(at) = at else {
            return false;
        };
        records.push(hand(need.name, at.seed()));
    }

    if utask::hatch(child).is_err() {
        return false;
    }

    // 收它的控制孔，再把记录推进去。
    let Some(up) = wait_quay(child) else {
        return false;
    };
    // SAFETY: `Pair` 是 `repr(C)`、尺寸由编译期断言锁死为 `PAIR_LEN`，故若干枚连排就是
    // 若干条记录、中间没有填充；只读这一段字节。
    let bytes = unsafe {
        core::slice::from_raw_parts(records.as_ptr().cast::<u8>(), PAIR_LEN * records.len())
    };
    up.push(bytes).is_ok()
}

/// 一条记录：名字 + 「种在子域表里」的那枚句柄。
fn hand(name: &str, token: PieToken) -> Pair {
    Pair::new(Name::new(name).expect("名字是编译期常量且装得下"), token)
}

/// 从配对块里按名字取句柄（块是逐条定长的一段，只读）。
fn pair_token(pairs: *const u8, count: usize, want: &str) -> Option<usize> {
    for i in 0..count {
        // SAFETY: 块是 boot 只读借映的一段（页对齐、`count * PAIR_LEN` 字节）；本域只读。
        // 用 `read_unaligned` 是因为记录步长 40 字节而块只保证页对齐。
        let rec = unsafe { core::ptr::read_unaligned(pairs.add(i * PAIR_LEN).cast::<Pair>()) };
        if rec.name().is_some_and(|n| n.as_str() == want) {
            return Some(rec.token().get());
        }
    }
    None
}

/// 等子域的控制孔：扫本域表里 `vestor == child` 的那一枚。
///
/// `collect` 是唯一的枚举手段。本域自己那二十来枚设备门闩的 `vestor` 是"无"（编码为
/// `TaskId(0)`），只有子域授回来的那枚才是它——**一个判据就够，不必约定位置**。
fn wait_quay(child: TaskId) -> Option<HolePie> {
    for _ in 0..QUAY_TRIES {
        let mut i = 0;
        loop {
            let Ok((token, _perm, vestor)) = mail::collect(i) else {
                return None;
            };
            // 越界的哨兵：这一遍扫完了。
            if token.get() == 0 {
                break;
            }
            if vestor == child {
                return Some(HolePie::from_token(token.get()));
            }
            i += 1;
        }
        // 还没来：让出处理器再扫（子域正在跑它的头几步）。
        let _ = starve();
    }
    None
}

/// 等目标结束。`join(tid, 0)` 返真 ⇒ **收尾已完成**（内核契约：退出钩子已跑完），
/// 故挂起过的那一次只当"醒了一次"，须复探。
fn join_done(tid: TaskId) {
    loop {
        if utask::join(tid, 0).unwrap_or(true) {
            return;
        }
        let _ = utask::join(tid, usize::MAX);
    }
}

/// 打一行。本域没有会话、没有控制台，调试面是唯一能说话的地方。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
