//! 管理端那一台：本景真正的控制者（身子在 `main.rs` 的引导镜像里）。
//!
//! 它自己备两份私有来源（迁移来源与控制页）、自己铸根、自己授子、把对端装成
//! **另一个 Team** 里的一台真任务，再把这一次的请求摆到共享控制页上、发起那一次
//! **真移交**。它在途窗口里那一手（放开对端、等它真登记）归内核夹具，本文件只报
//! 它前后两侧的读数，并按真实读数当场判。
//!
//! 判定权在它这一个跑点：夹具那一步不是 `armed`（多 Hart 只有一颗时必然到点），
//! 或者对端读到的不是第 0 页那份花纹，本景当场红——不把超时当证明。

use core::time::Duration;

use env::Access;
use env::EXIT_OK;
use env::ExitCause;
use env::JoinReply;
use env::Mark;
use env::PieToken;
use env::Policy;
use env::TaskId;
use env::UnitTarget;
use env::Wait;
use env::unit;
use resource::raw::Hole as RawHole;

use super::super::memory_transfer::console::Console;
use super::handshake;
use super::page::Page;
use super::shared::Shared;
use super::source::Source;
use crate::boot::Accounts;
use crate::boot::Catalog;
use crate::system::loader::Image;
use crate::system::loader::Loader;

/// 迁移来源挂在管理端空间里的起点（第 0 页在这一格，第 1 页是目标空槽）。
///
/// 刻意选高：对端那两条关系的地址由内核在自己空间里最低可用处定，两者数值上不会
/// 撞在一起，宿主就能把“本景的页错误”按 VA 对到具体那一条关系上。
const SOURCE_AT: usize = 0x7000_0000;
/// 等控制页上一格变样的总时限（纳秒）：到点就是本景失败，不挂在那里。
const LIMIT_NS: u64 = 10_000_000_000;
/// 等对端离场的总时限（纳秒）。
const JOIN_LIMIT_NS: u64 = 20_000_000_000;
/// 递子分支号那道孔的标号（对端按它在 sire 表里找回来）。
const LINK_MARK: Mark = Mark::of("memory-transfer-race-link");
/// 递号那一次的等。
const LINK_WAIT: Wait = Wait::AtMost(5000);

/// 引导镜像那一台：本景的管理端。
pub fn acceptance() {
    let accounts = Accounts::take().expect("race: boot accounts");
    let catalog = Catalog::of_boot(&accounts).expect("race: manifest");
    let image = catalog
        .find("memory-transfer-race-unit")
        .expect("race: the peer image");
    let console = console(&accounts);
    let mut loader = Loader::new();
    let built = loader
        .build(Image {
            bytes: image.elf,
            kind: image.kind,
        })
        .expect("race: build the peer");
    // 先起一台**被握住**的真任务：授子要一个真的 TaskId，而它加载完成前不会跑。
    let peer = built.spawn(&[], 0).expect("race: spawn the peer");
    // 两份私有来源，各有各的寿命：控制页与迁移范围**分开**。
    let mut source = Source::mint(2, SOURCE_AT, 1);
    let mut control = Shared::mint(handshake::VA);
    let page = Page::at(handshake::VA);
    page.reset();
    // 第 0 页先真物化并填上花纹（头一个字节走一次真缺页）。
    source.fill(0);
    // 授两枚子分支给对端，再把号经一道孔递过去：加载完成前它不会跑。
    let branches = [control.accord(peer), source.accord(peer)];
    hand_over(peer, branches);
    unit::embark_task(peer).expect("race: embark the peer");
    // 等对端把第 0 页读过、把坐标与读数报回来（它在自己空间里真发布了一条 PTE）。
    await_word(
        &console,
        "the peer never reported its page-0 read",
        || page.ready(),
        0,
    );
    note(
        &console,
        &alloc::format!(
            "phase=fault peer={} va0={:#x} va1={:#x} read0={:#x}",
            page.peer(),
            page.va0(),
            page.va1(),
            page.read0()
        ),
    );
    // 摆好请求，发起**真移交**：第 0 页 → 第 1 页（同一份根上的两个窗口）。
    page.request(handshake::mode::FAULT);
    let moved = source.shift(0, 1);
    let kernel = page.state_name();
    note(
        &console,
        &alloc::format!(
            "phase=fault transfer={moved:?} kernel={kernel} foreign={}",
            page.foreign()
        ),
    );
    assert!(
        moved.is_ok(),
        "race: the transfer did not commit: {moved:?}"
    );
    // 夹具那一步就是本景要证的那一件事：它在真在途窗口里**真看到了对端的登记**。
    // 多 Hart 只有一颗时它只能到点（`timeout`），本景当场红——那是负对照。
    assert_eq!(
        kernel, "armed",
        "race: the fixture never saw the peer register while the window was held"
    );
    // 夹具在窗口里当场读到的那一条：那条**别人空间**的关系真握着源页的翻译——
    // 这次移交的成功因此是“真从别人空间里撕下一格”，不是范围上的空话。
    assert_eq!(
        page.foreign(),
        1,
        "race: the fixture never saw a translation in the other space's relation"
    );
    // 对端被那一次移交自己放行，醒后读第 1 页并把读数写回。
    let read1 = await_word(
        &console,
        "the peer never read the moved page",
        || page.read1(),
        handshake::NONE,
    );
    note(
        &console,
        &alloc::format!("phase=fault read1={read1:#x} read0={:#x}", page.read0()),
    );
    assert_ne!(
        read1, 0,
        "race: the peer read a zero page: the carrier moved"
    );
    assert_eq!(
        read1,
        page.read0(),
        "race: the peer read a different page than the one it announced"
    );
    // 对端走的是真退出（入口那一台报收尾），管理端按真退出核它。
    let exit = join(peer);
    note(
        &console,
        &alloc::format!(
            "phase=fault joined cause={:?} reason={:#x}",
            exit.cause,
            exit.reason
        ),
    );
    assert_eq!(
        exit.cause,
        ExitCause::Reap,
        "race: the peer did not end by itself"
    );
    assert_eq!(exit.reason, EXIT_OK, "race: the peer failed");
    // 收尾：夹具不再插手，控制页与来源各自关掉（关系、引用、根一并交回）。
    page.retire();
    control.close();
    source.close();
    note(&console, "close done");
}

/// 开一道孔、只授出“能取”那一格，再把两枚子分支的号推进去。
///
/// 子分支的号已经在**它**表里才作数（`accord` 已把它放进对端表里），这里递的
/// 只是那两个号本身。
fn hand_over(peer: TaskId, branches: [PieToken; 2]) {
    let link = env::pie::unseal(env::UnsealArgs::hole(LINK_MARK)).expect("race: the link hole");
    ::resource::port::ship(link, peer, Access::FETCH, Policy::NONE)
        .expect("race: hand the link hole over");
    let mut bytes = [0u8; 2 * PieToken::WIDTH];
    bytes[..PieToken::WIDTH].copy_from_slice(&branches[0].to_bytes());
    bytes[PieToken::WIDTH..].copy_from_slice(&branches[1].to_bytes());
    RawHole::from_raw(link)
        .push(&bytes, LINK_WAIT)
        .expect("race: push the branch tokens");
}

/// 等控制页上一格离开 `pending` 那一值：到点当场红，并先把理由报一行。
///
/// 不睡固定时长、不拿“即将”当“已经”；两次查询之间只 `park` 一小段。
fn await_word(console: &Console, why: &str, read: impl Fn() -> u64, pending: u64) -> u64 {
    let until = env::chrono::clock() + LIMIT_NS;
    loop {
        let value = read();
        if value != pending {
            return value;
        }
        if env::chrono::clock() >= until {
            note(console, &alloc::format!("FAIL {why}"));
            panic!("race: {why}");
        }
        execution::room::park(Duration::from_millis(2)).expect("race: park");
    }
}

/// 等对端离场，取它为什么离场。
fn join(peer: TaskId) -> env::TaskExit {
    let until = env::chrono::clock() + JOIN_LIMIT_NS;
    loop {
        match unit::join(UnitTarget::Task(peer), Wait::AtMost(50), false) {
            Ok(JoinReply::Reaped(exit)) => return exit,
            Ok(JoinReply::Pending) => {
                assert!(env::chrono::clock() < until, "race: the peer never left")
            }
            Ok(JoinReply::Complete) => unreachable!("a task is not a team"),
            Err(error) => panic!("race: join: {:?}", error.source),
        }
    }
}

/// 本景输出那一面：串口那一页的门闩由**引导账**直接给（管理端自己就是引导镜像）。
///
/// 本景只出不进（与对端的握手全在共享控制页上），故没有宿主门。
fn console(accounts: &Accounts) -> Console {
    let region = Console::region(accounts);
    let token = accounts.token(region).expect("race: the console lock");
    Console::open(token)
}

/// 报一行本景的话（串口那一面，宿主按前缀认）。
fn note(console: &Console, text: &str) {
    console.put(alloc::format!("memory-transfer-race: {text}\n").as_bytes());
}
