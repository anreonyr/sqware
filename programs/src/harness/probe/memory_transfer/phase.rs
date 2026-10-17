//! 三个阶段：一次填回放行两名 · 杀一名留下另一名 · 撤一页杀一名并让另一名重新入队。
//!
//! 每个阶段的形状一样：备来源 → **先物化填花纹** → 把承载移出去造成 `Missing` →
//! 在同一 Team/Space 里起两名故障任务 → 报就绪（阶段名、View、tid）→ 收门 →
//! 做本阶段那一下。控制者**在填回之前不再自己访问那段 View**。
//!
//! 三个阶段各自的来源**一直持到全跑完**（`unit` 里一并收尾）：这样每阶段的 View 落在
//! 不同的地址上——宿主就能把“本阶段的页错误”按 VA 与已知 tid 对到本阶段，
//! 而不是三个阶段的读数搅在一起。

use alloc::format;

use env::{EXIT_OK, ExitCause, JoinReply, TaskId, TeamId, UnitTarget, Wait};
use execution::unit::task::{Join, try_spawn};

use super::fault;
use super::gate::Gate;
use super::source::Source;

/// 单次“等一条结束”的总时限（纳秒）。
const EXIT_LIMIT_NS: u64 = 20_000_000_000;

/// `Slay` 那条路的出口码（内核 `messenger::EXIT_DOOM`）：**不在 ABI 里**，
/// 故本景只拿它当“这一名确实是被叫走的”那一个读数，不当成对外承诺。
const DOOM_REASON: usize = 0xFFFF_FFFE;

/// A：一条 View 上同一页两名等待者，恰一次填回，一个 signal 放行两名。
pub fn a(gate: &Gate) -> Source {
    let source = Source::acquire(2, 2);
    fault::fill(0, source.page(0));
    source.shift(0, 1);
    let one = reader(0, source.page(0));
    let two = reader(0, source.page(0));
    gate.ready(
        "A",
        source.page(0),
        source.id().get(),
        [one.id().get(), two.id().get()],
    );
    gate.await_byte("A", b'A');
    // 本阶段**恰一次**填回：这一手之前不碰那条 View，之后也不再碰它。
    source.shift(1, 0);
    gate.note("phase=A filled");
    let want = fault::sum(0);
    assert_eq!(one.join(), want, "memory-transfer: the first reader's page");
    assert_eq!(
        two.join(),
        want,
        "memory-transfer: the second reader's page"
    );
    gate.note("phase=A done");
    source
}

/// B：只叫走**一名**等待者（不是 `Doom` 整 Team），另一名照样等在那条 View 上，
/// 事后才被填回放行——它的收工不靠空间死亡清理。
pub fn b(gate: &Gate) -> Source {
    let source = Source::acquire(2, 2);
    fault::fill(0, source.page(0));
    source.shift(0, 1);
    let gone = doomed(0, source.page(0));
    let keep = reader(0, source.page(0));
    gate.ready(
        "B",
        source.page(0),
        source.id().get(),
        [gone.get(), keep.id().get()],
    );
    gate.await_byte("B", b'B');
    env::unit::slay_task(gone)
        .unwrap_or_else(|error| panic!("memory-transfer: slay: {:?}", error.source));
    let exit = exit_of(gone);
    assert_eq!(exit.cause, ExitCause::Slay, "memory-transfer: B slay cause");
    assert_eq!(exit.reason, DOOM_REASON, "memory-transfer: B slay reason");
    // 另一名**没完工**，而这一条空间还活着（下面它靠填回被唤醒、正常收工）。
    assert!(
        !env::unit::join_task(keep.id(), Wait::POLL).expect("memory-transfer: poll the keeper"),
        "memory-transfer: the keeper left with the slain one"
    );
    gate.note(&format!("phase=B slain tid={}", gone.get()));
    gate.await_byte("B2", b'b');
    source.shift(1, 0);
    gate.note("phase=B filled");
    assert_eq!(
        keep.join(),
        fault::sum(0),
        "memory-transfer: the keeper's page"
    );
    gate.note("phase=B done");
    source
}

/// C：一条两页 View 上两页都 `Missing`；只撤第一页那一页的关系——
/// 那一页上的等待者当场被真终止，另一页上的**重新入队**；随后填回第二页它才收工。
pub fn c(gate: &Gate) -> Source {
    let source = Source::acquire(4, 2);
    fault::fill(0, source.page(0));
    fault::fill(1, source.page(1));
    source.shift(0, 2);
    source.shift(1, 3);
    let head = doomed(0, source.page(0));
    let tail = reader(1, source.page(1));
    gate.ready(
        "C",
        source.page(0),
        source.id().get(),
        [head.get(), tail.id().get()],
    );
    gate.await_byte("C", b'C');
    source.withdraw(0);
    let exit = exit_of(head);
    assert_eq!(
        exit.cause,
        ExitCause::Fault,
        "memory-transfer: C fault cause"
    );
    gate.note(&format!("phase=C killed tid={}", head.get()));
    gate.await_byte("C2", b'c');
    source.shift(3, 1);
    gate.note("phase=C filled");
    assert_eq!(
        tail.join(),
        fault::sum(1),
        "memory-transfer: the tail's page"
    );
    gate.note("phase=C done");
    source
}

/// 起一名故障任务：它一进去就读第 `page` 页，真正的访存（与随后的等待）就在那里，
/// 交回整页花纹的和供控制者核对。
fn reader(page: usize, base: usize) -> Join<u64> {
    try_spawn(move || fault::check(page, base))
        .unwrap_or_else(|error| panic!("memory-transfer: spawn reader: {:?}", error.source))
}

/// 起一名**注定被叫停**的故障任务：raw 入口 ＋ 标量参数，不铸闭包也不铸结果槽。
///
/// 故意终止那一条路上没有任何 Rust 展开（ `Slay` / 页错误直接换任务），
/// 闭包与结果容器在那条路上没人放；故这两名走 `Spawn(team 0)` 的标量参数面
/// （与 `probe-terminal` 那台自建任务同一条），共享空间里不留用户堆残留。
fn doomed(page: usize, base: usize) -> TaskId {
    let args = [page, base];
    let task = execution::unit::spawn(TeamId::new(0), reader_entry as *const () as usize, &args, 0)
        .unwrap_or_else(|error| panic!("memory-transfer: spawn doomed: {:?}", error.source));
    env::unit::embark_task(task)
        .unwrap_or_else(|error| panic!("memory-transfer: embark doomed: {:?}", error.source));
    task
}

/// 那条 raw 入口（`a0` = 本任务栈上的启动参数区，与 `execution::unit::task::trampoline` 同形）：
/// 读一次、核一遍，没被叫停就自己正常收工——被叫停那两种由控制者按回执核。
/// 不建立 TLS；预期终止前只读启动参数与首个 volatile 字节，不分配。
extern "C" fn reader_entry(arg: usize) -> ! {
    // SAFETY: `Spawn` 把两个字写在本任务启动参数区，故从 `arg` 起读两个字是这一次调用的契约。
    let args = unsafe { core::slice::from_raw_parts(arg as *const usize, 2) };
    let _ = fault::check(args[0], args[1]);
    execution::room::reap(EXIT_OK, None)
}

/// 等一名故障任务结束，取它为什么结束。
fn exit_of(task: TaskId) -> env::TaskExit {
    let until = env::chrono::clock() + EXIT_LIMIT_NS;
    loop {
        match env::unit::join(UnitTarget::Task(task), Wait::AtMost(50), false) {
            Ok(JoinReply::Reaped(exit)) => return exit,
            Ok(JoinReply::Pending) => assert!(
                env::chrono::clock() < until,
                "memory-transfer: task {} never left",
                task.get()
            ),
            Ok(JoinReply::Complete) => unreachable!("a task is not a team"),
            Err(error) => panic!("memory-transfer: join: {:?}", error.source),
        }
    }
}
