#![no_std]
#![no_main]

//! probe-deep — **深度那一格的证客**：一层层往下 `part`，量出持树者死在哪一层。
//!
//! 树有四条**按深度递归**的私有助手（`look` / `holds` / `take` / `put_in`），而一台域的栈是
//! `TASK_STACK_SIZE = 16 KiB`。广度有闸（`PANE_CAP = 16`）、一条路有闸（`ROAD_MAX = 8`），
//! **深度一个闸都没有**——故"往下打"这一手能直接把持树者从栈上打下去。
//!
//! ```text
//!   part(root, "d0") → id0      （持树者递归 1 层）
//!   part(id0,  "d1") → id1      （递归 2 层）
//!   …
//!   一直到来不及答（持树者栈溢出死掉 ⇒ 这一问超时）
//! ```
//!
//! # 这一台**不进 soak**（照实记）——它现在是**装好了但不上电**的
//!
//! 它是炸弹：一跑，命名空间就没了——后面每一台客人的 `seek`/`find` 全都没人答。故它挂在
//! `INITRD_BINS` 上（**装**），但**不在装配单里**（**不上电**）；量死线那一次单独把它塞进
//! 装配单跑一轮，量完就撤。等树的寻址改成**按号直达**（②那一刀：号就是槽的下标，四条递归
//! 助手一并消失）之后，同一台探针**反过来是正证**：打到几百层每一手都答得出——那时它才进
//! 装配单与 soak。
//!
//! # 量出来的数（2026-09-24，一次冷启）
//!
//! ```text
//!   operator: tid=3                                   ← 持树者（临时加的一行自报，量完撤）
//!   probe-deep: alive at 32 / 64 / 96
//!   [ERROR] reserved region access: Store at VA(0x1bff8), pc=0x10028
//!   user fault killed: tid=3 cause=15 stval=0x1bff8   ← **持树者自己被杀**
//!   probe-deep: tree tid=21 last=116 cap=512 code=7 after=err:7
//!                                     ↑ 第 117 手答不出   ↑ 连最浅那一手也答不出：命名空间没了
//! ```
//!
//! **第 117 层死**（`last=116` 成，第 117 手把它打下去）。三格都量到了：
//!
//! 1. **死的是持树者自己**（`tid=3`，用一行临时自报对上的名字）；
//! 2. **死法不是"答一格负码"，是整台域被杀**（`user fault killed`）——16 KiB 任务栈 ×
//!    117 帧 ≈ 每帧 140 字节，正好见底；
//! 3. **后果是命名空间整个消失**（`after=err:7`：失败之后再打一手最浅的也答不出）。
//!
//! 故"往下打"这一手是**任何一台已绑身份的客人**都做得成的、代价极小的 DoS。数目字本身不是
//! 判据（帧大小会随代码漂），**"改前会死、改后不死"**才是。

// 本文件是一份**独立的 bin**（`programs/Cargo.toml` 的 `prog-probe-deep`），**不进 lib**
// ——与 `echo` / `probe-rule` 同一条：`programs/src/user/mod.rs` 里没有它。
//
// 两条 `extern crate` 缺一不可（实测）：`alloc` 是 `format!` 要用；`programs` **不是**为了
// 用它里面的东西，而是为了把 `libprograms` 链进来——**panic handler 与 `_start` 都住那份
// lib**（`programs/src/entry.rs`）。少了它，链接期报 `` `#[panic_handler]` function required ``。
extern crate alloc;
extern crate programs;

use alloc::format;
use alloc::string::String;

use env::Name;
use protocol::operator::Where;
use protocol::operator::call as ocall;
use protocol::operator::client as operator;
use runtime::env::debug;
use runtime::env::room::exit_with_note;
use runtime::env::unit as utask;

/// 一趟的总上限（毫秒）。**必须有界**：持树者死在第 N 层时，第 N+1 手要**及时**答出来。
const MS: usize = 1000;

/// 打到多少层就收手（还没死的话）。
const MAX_DEPTH: usize = 512;

/// 每几层报一行（免得刷屏——但"活着"这件事要看得见进度）。
const STEP: usize = 32;

/// 退场码：打到收手 / 中途那一手答不出（都不是 panic）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

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
    let Ok(me) = utask::self_id() else {
        bail("probe-deep: no self id")
    };

    // 从根往下：每一层一个名字、一块一格宽的 `Pane`。名字用层号，故不会撞名。
    let mut at = Where::Root;
    let mut last = 0usize;
    let mut code = ocall::OK;
    while last < MAX_DEPTH {
        let Ok(one) = Name::new(&format!("d{last}")) else {
            bail("probe-deep: bad name")
        };
        match operator::part(talk, &tree, at, one, MS) {
            Ok(id) => {
                at = Where::At(id);
                last += 1;
                if last % STEP == 0 {
                    say(&format!("probe-deep: alive at {last}"));
                }
            }
            Err(one) => {
                code = one;
                break;
            }
        }
    }

    // **死线之后树还活着吗**：再打一手最浅的（根下）。这一问把"树死了"与"树没事、只是刚才
    // 那一手答不出"分开——量深度这一刀最要紧的一格就是它。
    let after = match Name::new("after") {
        Ok(one) => match operator::part(talk, &tree, Where::Root, one, MS) {
            Ok(id) => format!("ok:{}", id.get()),
            Err(one) => format!("err:{one}"),
        },
        Err(_) => String::from("badname"),
    };

    // 一行读数：**本域是谁**、最后一层成的是第几层、收手的上限、下一手答的码、以及树还在不在。
    say(&format!(
        "probe-deep: tree tid={} last={last} cap={MAX_DEPTH} code={code} after={after}",
        me.get()
    ));

    let survived = last == MAX_DEPTH;
    exit_with_note(
        if survived { E_OK } else { E_TRIP },
        if survived {
            "probe-deep: holder survived to the cap"
        } else {
            "probe-deep: a hand stopped answering"
        },
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
