//! 两阶段的读者：一头是被内核放开的**真实等待者**，另一头是失败之后的**晚到者**。
//!
//! 两者读的是同一件东西：目标页那一张**新清零页**。差别只在起手——等待者先在普通共享
//! 标志上自旋（不起 Park/Alarm），被内核那一个在途夹具放开之后才真的访存，故那一次读
//! 落在目标关系的 View 等待上；晚到者在失败结算之后才起，故它直接物化。
//!
//! 两台的收工与读数都不经父子结果槽：读者只把读数写回握手页，再正常收工。
//! 自旋里不取时钟、不 park：本景不许把另一种等待印在同一具 tid 上。

use env::{PAGE_SIZE, TaskId, TeamId};
use execution::room;

use super::Console;
use super::handshake::RELEASE;
use super::note;
use super::page;

/// 起一头等待者：它在目标关系上真的等过一次（宿主按真实 `room.wait` 认）。
pub fn waiter(console: &Console, base: usize) -> TaskId {
    start(console, base, true)
}

/// 起一头晚到者：失败结果完成之后才读目标页。
pub fn late(console: &Console, base: usize) -> TaskId {
    start(console, base, false)
}

/// 起一台 raw 任务（标量参数，不铸闭包也不铸结果槽）。
fn start(console: &Console, base: usize, wait: bool) -> TaskId {
    let args = [base, wait as usize];
    let task = match execution::unit::spawn(TeamId::new(0), entry as *const () as usize, &args, 0) {
        Ok(task) => {
            note(console, &alloc::format!("spawn ok tid={}", task.get()));
            task
        }
        Err(error) => {
            note(console, &alloc::format!("spawn ERR {error:?}"));
            panic!("memory-transfer-fresh: spawn the reader");
        }
    };
    match env::unit::embark_task(task) {
        Ok(()) => note(console, &alloc::format!("embark ok tid={}", task.get())),
        Err(error) => {
            note(
                console,
                &alloc::format!("embark ERR {error:?} tid={}", task.get()),
            );
            panic!("memory-transfer-fresh: embark the reader");
        }
    }
    task
}

/// 读者的身子：`a0` = 本任务栈上的启动参数区（`[base, wait]`，与
/// `execution::unit::task::trampoline` 同形）。
///
/// 不建 TLS、不分配：预期终止前只读启动参数、握手页与目标页，最后写回读数收工。
extern "C" fn entry(arg: usize) -> ! {
    // SAFETY: `Spawn` 把两个字写在本任务启动参数区，故从 `arg` 起读两个字是这一次调用的契约。
    let args = unsafe { core::slice::from_raw_parts(arg as *const usize, 2) };
    let (base, wait) = (args[0], args[1] != 0);
    if wait {
        page::mark_ready();
        while page::start() != RELEASE {
            core::hint::spin_loop();
        }
    }
    page::publish_verdict(zero_page(base));
    room::reap(env::EXIT_OK, None)
}

/// 读整页并要求它是一张新清零页；头一个字节就是那一次会等待的访存。
///
/// 内容判据是**逐字节读到 0**（与 `memory_transfer::fault::check` 同一条：读一次比一次）；
/// 交回的和只是同一个事实的读数，供控制者打上控制台，不是另一条独立判据。
fn zero_page(base: usize) -> u64 {
    // SAFETY: 目标页在本 Team 的映射里；这一次读就是本阶段要观察的那一次访存。
    let head = unsafe { core::ptr::read_volatile(base as *const u8) };
    assert_eq!(head, 0, "memory-transfer-fresh: a fresh page head byte");
    let mut total = head as u64;
    for offset in 1..PAGE_SIZE {
        // SAFETY: 同上。
        let got = unsafe { core::ptr::read_volatile((base + offset) as *const u8) };
        assert_eq!(
            got, 0,
            "memory-transfer-fresh: byte {offset} of a fresh page"
        );
        total += got as u64;
    }
    total
}
