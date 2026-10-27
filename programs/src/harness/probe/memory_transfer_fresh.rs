//! Fresh 失败回退那一景：一次真移交失败把目标还成**从未物化的 Fresh**，
//! 被放行的真实等待者与晚到者物化的都是**一张新清零页**。
//!
//! 身子只有一台（引导镜像那一台，与 `memory-transfer-tlb` 同款）：它自己就是控制者，
//! 在同一 Team／空间里起真任务。两阶段的形状一样：
//!
//!   备两份**不同**的普通来源（各一条 View）＋ 一页握手页（第三个 Flat，定点挂上） →
//!   把源页物化并发布（真压着一条 PTE） → 目标**从未物化**（Fresh） →
//!   把这一次的请求写进握手页 → **真移交**（内核在占槽之后注入一次撤页确认失败） →
//!   结果必须是 `Busy`，且两端按原态还回（源仍 Resident、目标仍 Fresh）。
//!
//! A：失败那一刻真有一名等待者登记在**目标关系**上（宿主按真实 `room.wait` 认）；
//!    被放行后它物化的是一张新清零页；源页那一边读回的是**原花纹**——新分配的空页
//!    只会读到 0，故那一次回退保住的是原承载的内容（**承载身份本身不在本景观测范围**）。
//! B：失败那一刻没有任何等待者，目标仍是从未物化的 Fresh；失败结果完成之后才起
//!    晚到读者，它直接物化一张新清零页，全程不产生 View 等待。
//!
//! 判定权在宿主：控制台那一面只报事实（阶段、VA、关系身份、tid、读数），结构化导出
//! 那一面给出真实的 `room.wait`/`room.wake`/`memory.page_fault`。控制者**不睡固定时长**、
//! 不拿「即将访存」的标志冒充入队：放开读者那一手在内核的在途夹具里，本文件只报
//! 它放开前后的读数。读者那一台不经父子结果槽交回：它把读数写回握手页，收工与
//! 回收由宿主按真实事件认。

// 握手页的线格式由 `kernel-test-api` 的 `memory_transfer_fresh` 提供：两侧按同一份
// 公开线格式读写。
use kernel_test_api::memory_transfer_fresh as handshake;
mod page;
mod reader;

use alloc::format;

use env::{MemoryFail, PAGE_SIZE, PieToken, TeamId, ViewControl};
use execution::memory::View;

use super::memory_transfer::console::Console;
use super::memory_transfer::fault;
use super::memory_transfer::source::Source;
use crate::boot::Accounts;

/// 等握手页上一格变样的时限（纳秒）：到点就是本景失败，不挂在那里。
const LIMIT_NS: u64 = 10_000_000_000;

/// 一次 `Attach` 拿到的当前权限：可读可写。
///
/// 与 `memory_transfer::source` 同一格线上事实：`Access` 在线上就是那几位（读 | 写）。
const READ_WRITE: u64 = 6;

/// 引导那一台：本景唯一的控制者。
///
/// 四个端点一直持到全跑完（与 D1 同款）：每阶段的 View 因此落在不同地址上，
/// 宿主就能把“这一阶段的页错误”按 VA 对到本阶段。
pub fn acceptance() {
    let accounts = Accounts::take().expect("memory-transfer-fresh: boot accounts");
    let console = console(&accounts);
    let mut meta = metadata();
    page::reset();
    let mut a_source = Source::acquire(1, 1);
    let mut a_target = Source::acquire(1, 1);
    let mut b_source = Source::acquire(1, 1);
    let mut b_target = Source::acquire(1, 1);
    phase_a(&console, &a_source, &a_target);
    phase_b(&console, &b_source, &b_target);
    page::retire();
    for one in [&mut b_target, &mut b_source, &mut a_target, &mut a_source] {
        one.close();
    }
    meta.close().unwrap_or_else(|error| {
        panic!("memory-transfer-fresh: close the handshake page: {error:?}")
    });
    note(&console, "close done");
}

/// A：失败那一刻真有一名等待者等在目标关系上，源页那一面内容原样。
fn phase_a(console: &Console, source: &Source, target: &Source) {
    fault::fill(0, source.page(0));
    let reader = reader::waiter(console, target.page(0));
    note(
        console,
        &format!(
            "phase=A source={:#x} target={:#x} view={} reader={}",
            source.page(0),
            target.page(0),
            target.id().get(),
            reader.get()
        ),
    );
    await_ready(console);
    page::request(handshake::mode::AWAIT, target.id().get());
    let failed = refuse(source, target);
    note(
        console,
        &format!("phase=A transfer={failed:?} kernel={}", page::state_name()),
    );
    let value = await_verdict(console);
    note(
        console,
        &format!("phase=A reader={} value={value:#x}", reader.get()),
    );
    // 源页那一边：那一次失败的撤页把它的 PTE 撤了，重新读一次拿到的是**原花纹**。
    // 新分配的空页只会读到 0，故这里读出花纹即内容没换承载；承载身份不在这里观测。
    let kept = fault::check(0, source.page(0));
    note(
        console,
        &format!("phase=A source={:#x} sum={kept:#x}", source.page(0)),
    );
}

/// B：失败那一刻没有任何等待者；失败结果完成之后才起晚到读者。
fn phase_b(console: &Console, source: &Source, target: &Source) {
    fault::fill(0, source.page(0));
    note(
        console,
        &format!(
            "phase=B source={:#x} target={:#x} view={}",
            source.page(0),
            target.page(0),
            target.id().get()
        ),
    );
    page::request(handshake::mode::LATE, target.id().get());
    let failed = refuse(source, target);
    note(
        console,
        &format!("phase=B transfer={failed:?} kernel={}", page::state_name()),
    );
    let late = reader::late(console, target.page(0));
    note(console, &format!("phase=B spawn tid={}", late.get()));
    let value = await_verdict(console);
    note(
        console,
        &format!("phase=B late={} value={value:#x}", late.get()),
    );
}

/// 这一手真移交**必须失败**：夹具没武装或没认这一景时它就会成功，
/// 那正是本景该红的地方（不在这里把失败咽下去）。
fn refuse(source: &Source, target: &Source) -> MemoryFail {
    execution::memory::transfer(source.handle(), 0, target.handle(), 0)
        .err()
        .unwrap_or_else(|| panic!("memory-transfer-fresh: the transfer committed"))
        .source
}

/// 等读者真的在自旋了——只报“它在跑”，不当入队证据（那是宿主的活）。
fn await_ready(console: &Console) {
    let _ = await_word(console, "the reader never started", page::ready, 0);
}

/// 等读者把读数写回握手页：到点没见就是本景失败。
fn await_verdict(console: &Console) -> u64 {
    await_word(
        console,
        "the reader never reported",
        page::verdict,
        handshake::VERDICT_NONE,
    )
}

/// 自旋等握手页上的一格离开 `pending` 那一值；到点当场红。
fn await_word(console: &Console, why: &str, read: fn() -> u64, pending: u64) -> u64 {
    let until = env::chrono::clock() + LIMIT_NS;
    let mut spins = 0u32;
    loop {
        let value = read();
        if value != pending {
            return value;
        }
        spins = spins.wrapping_add(1);
        if spins % 4096 == 0 && env::chrono::clock() >= until {
            note(console, &format!("FAIL {why}"));
            panic!("memory-transfer-fresh: {why}");
        }
        core::hint::spin_loop();
    }
}

/// 握手页：第三个 Flat，定点挂在固定地址上（内核夹具按同一个数寻址）。
fn metadata() -> View {
    execution::memory::map(
        TeamId::new(0),
        handshake::VA,
        PAGE_SIZE,
        PieToken::NONE,
        0,
        READ_WRITE,
        ViewControl::Caller,
    )
    .unwrap_or_else(|error| panic!("memory-transfer-fresh: the handshake page: {error:?}"))
}

/// 本景输出那一面：串口那一页的门闩由引导账直接给（控制者自己就是引导镜像）。
fn console(accounts: &Accounts) -> Console {
    let region = Console::region(accounts);
    let token = accounts
        .token(region)
        .expect("memory-transfer-fresh: the console lock");
    Console::open(token)
}

/// 报一行本景的话（串口那一面，宿主按前缀认）。
fn note(console: &Console, text: &str) {
    console.put(format!("memory-transfer-fresh: {text}\n").as_bytes());
}
