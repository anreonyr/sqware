//! 真实 Task 等待链：一条 View 上的缺页等待、一次移交填回、三条生命周期通知。
//!
//! 这一景两台：**装载者**（引导镜像那一台，`acceptance`）只做三件事——按设备树把
//! 控制台那一枚区门闩取出来、把控制单元装成一台真任务（故那一台有 sire）、
//! 把门闩授过去（授出那一枚，再把号经一道孔递过去）；
//! **控制单元**（`unit()`）才是控制者：它自己备来源、自己造 View、
//! 在同一 Team/Space 里起故障任务、按宿主那一个字节的门逐阶段推进。
//!
//! 为什么控制者必须是被装载出来的一台：故障任务与它同 Team，故那些任务带着那条
//! Team 的**收据**（owner = 装载者）——按页错误真终止的任务因此只记自己的出口，
//! 不会被当成「无人认领的塌方」；引导队那一台（owner = 0）没有收据。
//! 代价是这一景不靠整景收尾那条账兜住团队里的意外塌方，改由控制者自己核：
//! 每一名故障任务都必须按本阶段预期的那一种离场（正常 `Reap`、`Slay`、`Fault`），
//! 否则下面的 `join` 或那一条断言当场失败，装载者再按“控制单元离场不对”把整景判红。
//!
//! 判定权在宿主：结构化诊断里看到本阶段该看到的真实事件之前，门不会开。
//! 程序侧不睡固定时长、不拿「即将访存」的标志冒充已入队；收不到门就是失败。

// 三份身子在**同族的另一个景**（`memory_transfer_tlb`）里也要用：那条景同样要一段私有来源、
// 同一份花纹与同一台串口。故只在本族（`probe`）之内放开模块名，不是对外接口。
pub(in crate::harness::probe) mod console;
pub(in crate::harness::probe) mod fault;
mod gate;
mod phase;
pub(in crate::harness::probe) mod source;

use crate::boot::{Accounts, Catalog};
use crate::system::loader::{Image, Loader};
use ::resource::raw::Hole as RawHole;
use env::{EXIT_OK, ExitCause, JoinReply, Permission, PieToken, TaskId, Wait, unit};

/// 等控制单元跑完的总时限（纳秒）：宿主那一侧另有自己的时限。
const JOIN_LIMIT_NS: u64 = 40_000_000_000;

/// 装载者与那一台之间只走一件东西：控制台门闩在**它那张表里**的号。
///
/// 它不能赊 `establish::find`：那一问比的是**资源自己的 owner**，而区那一枚是引导期
/// 以 `TaskId(0)` 铸的、不是这台装载者——故号只能当数据过一道孔。
const LINK_MARK: env::Mark = env::Mark::of("memory-transfer-link");

/// 装载者与那一台之间那一次交接的等。
const LINK_WAIT: Wait = Wait::AtMost(5000);

/// 引导镜像那一台：装载控制单元，把控制台门闩授给它，等它跑完。
///
/// 控制单元不是自己报「跑完了」就算数——装载者核对**它为什么离场**：
/// 只有自愿、正常的那一种（`Reap` + `EXIT_OK`）才放行本景的成功标记。
pub fn acceptance() {
    let accounts = Accounts::take().expect("memory-transfer: boot accounts");
    let catalog = Catalog::of_boot(&accounts).expect("memory-transfer: manifest");
    let image = catalog
        .find("memory-transfer-unit")
        .expect("memory-transfer: the control unit image");
    let region = console::Console::region(&accounts);
    let console = accounts
        .token(region)
        .expect("memory-transfer: the console region lock");
    let mut loader = Loader::new();
    let unit = loader
        .build(Image {
            bytes: image.elf,
            kind: image.kind,
        })
        .expect("memory-transfer: build the control unit");
    let task = unit
        .spawn(&[], 0)
        .expect("memory-transfer: spawn the control task");
    // 先授出那一枚（号在**它**表里才作数），再把号经孔递过去：加载完成前它不会跑。
    let seed = accord_console(console, task);
    hand_over(task, seed);
    unit::embark_task(task).expect("memory-transfer: embark the control task");
    let exit = join(task);
    crate::debug::put(&alloc::format!(
        "memory-transfer: control task tid={} cause={:?} reason={:#x}",
        exit.task.get(),
        exit.cause,
        exit.reason
    ));
    assert_eq!(
        exit.cause,
        ExitCause::Reap,
        "memory-transfer: the control task did not end by itself"
    );
    assert_eq!(
        exit.reason, EXIT_OK,
        "memory-transfer: the control task failed"
    );
}

/// 控制单元那一台：本景真正的控制者。
pub fn unit() {
    let link = ipc::session::establish::find(unit::sire(), LINK_MARK)
        .expect("memory-transfer: the link hole from the loader");
    let mut bytes = [0u8; 8];
    let (len, _) = RawHole::from_raw(link)
        .pull(&mut bytes, LINK_WAIT)
        .expect("memory-transfer: the console lock from the loader");
    assert_eq!(len, bytes.len(), "memory-transfer: a half link frame");
    let token = PieToken::from_bytes(&bytes).expect("memory-transfer: the console lock bytes");
    let gate = gate::Gate::new(console::Console::open(token));
    // 三阶段的来源都持到全跑完才收尾：每阶段的 View 因此落在不同地址上，
    // 宿主就能按 VA 把“本阶段的页错误”与已知 tid 对到本阶段。
    let mut held = [phase::a(&gate), phase::b(&gate), phase::c(&gate)];
    for source in held.iter_mut().rev() {
        source.close();
    }
    gate.note("phase=close done");
}

/// 把一枚门闩授给控制单元，并打上本景那一个标号（它按标号在**自己**表里找回来）。
///
/// 只给读与写：控制台那一页要读寄存器状态、也要写发送格；不授转授那一格（`VEST`）。
/// 区那一枚是**独占**（`ONLY`）发的，而授出的子集必须与它同形（内核 `grant::form_ok`）
/// ——故 `ONLY` 原样留着，不分给第二枚。
fn accord_console(token: PieToken, task: TaskId) -> PieToken {
    let subset = Permission::FETCH | Permission::STORE | Permission::ONLY;
    env::pie::accord(token, task, subset, console::UART_MARK)
        .expect("memory-transfer: hand the console to the control task")
}

/// 开一道孔、只授出「能取」那一格，再把号推进去：那一台按标号找到这道孔、取下八个字节。
fn hand_over(task: TaskId, seed: PieToken) {
    let link =
        env::hole::unseal(LINK_MARK, env::HoleLimits::default()).expect("memory-transfer: the link hole");
    ::resource::port::ship(link, task, env::Access::FETCH, env::Policy::NONE)
        .expect("memory-transfer: hand the link hole over");
    RawHole::from_raw(link)
        .push(&seed.to_bytes(), LINK_WAIT)
        .expect("memory-transfer: push the console lock");
}

/// 等控制单元离场，取它为什么离场。
///
/// 有一道总时限：连着不答就当本景失败，而不是把两台一起挂在那里。
fn join(task: TaskId) -> env::TaskExit {
    let until = env::chrono::clock() + JOIN_LIMIT_NS;
    loop {
        let reply = env::unit::join(env::UnitTarget::Task(task), Wait::AtMost(50), false)
            .unwrap_or_else(|error| panic!("memory-transfer: join failed: {:?}", error.source));
        match reply {
            JoinReply::Reaped(exit) => return exit,
            JoinReply::Pending => {
                assert!(
                    env::chrono::clock() < until,
                    "memory-transfer: the control task never left"
                );
            }
            JoinReply::Complete => unreachable!("a task is not a team"),
        }
    }
}
