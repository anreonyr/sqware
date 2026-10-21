//! 对端那一台：由管理端装载进**另一个 Team**（另一个 Space）。
//!
//! 它在**自己那个空间**里把两条子分支各弄成一条关系：来源那条盖两页
//! （第 0 页要搬、第 1 页是目标空槽），控制页那条盖一页——但控制页那条挂在
//! **定点** [`handshake::VA`] 上（内核夹具按同一个数寻址，与它那种定点读法同址）。
//! 程序序：
//!
//! 1. 先读一次第 0 页——那一手真物化、真在这条关系上发布一条 PTE，
//!    然后把 tid、两个 VA 与那一页的读数报回控制页。
//! 2. 等在 `START` 上：那一枚字由内核夹具在**占槽之后、撤 PTE 之前**放开。
//! 3. 去读第 1 页。它此刻是移交留的空位（`Reserved`、没有 PTE），
//!    故这一次读是一次真冷缺页，按这段关系整段等一次；被那一次移交自己放行之后
//!    重试同一条指令，读到的就是搬过来的那份承载。
//! 4. 等 `MODE=TEARDOWN`：管理端摆好第二段请求后，它交出**自己那枚来源子分支**
//!    （`Release`，真入口）。那一次 `Cleanup::finish` 会在内核夹具那里停一下，
//!    由**在途窗口**那一手放行；控制页那条关系**不动**，故它还能把结果报回去。
//!
//! 收工不经任何结果槽：它把读数与 `Release` 的结果写回控制页就正常返回，
//! 由入口那一台报收尾。

use env::AcquireSource;
use env::PAGE_SIZE;
use env::PieToken;
use env::Placement;
use env::ReleaseMode;
use env::TeamId;
use env::ViewControl;
use env::VirtAddr;
use env::Wait;
use env::unit;
use execution::memory::Reference;
use resource::raw::Hole as RawHole;

use super::super::memory_transfer::fault;
use super::handshake;
use super::page::Page;
use super::source::READ_WRITE;

/// 管理端递子分支号那道孔的标号（对端按它在 sire 表里找回来）。
const LINK_MARK: env::Mark = env::Mark::of("memory-transfer-race-link");
/// 收号那一次的等。
const LINK_WAIT: Wait = Wait::AtMost(5000);

/// 对端那一台的身子。
pub fn run() {
    let link = ipc::session::establish::find(unit::sire(), LINK_MARK)
        .expect("race: the link hole from the manager");
    let mut bytes = [0u8; 2 * PieToken::WIDTH];
    let (len, _) = RawHole::from_raw(link)
        .pull(&mut bytes, LINK_WAIT)
        .expect("race: the branch tokens from the manager");
    assert_eq!(len, bytes.len(), "race: a half link frame");
    let control =
        PieToken::from_bytes(&bytes[..PieToken::WIDTH]).expect("race: the control branch bytes");
    let source =
        PieToken::from_bytes(&bytes[PieToken::WIDTH..]).expect("race: the source branch bytes");
    // 控制页那条子分支挂在**定点**上：本端从它自己的分支取得一份引用，再按
    // `Placement::exact` 映射到自己空间里那个固定地址（仍走真实 Reference/Attach）。
    let reference = Reference::acquire(
        AcquireSource::Source {
            token: control,
            offset: 0,
            count: 1,
        },
        PieToken::NONE,
    )
    .unwrap_or_else(|error| panic!("race: control reference: {:?}", error.source));
    let placement =
        Placement::exact(VirtAddr::new(handshake::VA), PAGE_SIZE).expect("race: control placement");
    let view = reference
        .attach(
            TeamId::new(0),
            0,
            1,
            placement,
            READ_WRITE,
            ViewControl::Caller,
        )
        .unwrap_or_else(|error| panic!("race: control map: {:?}", error.source));
    assert_eq!(
        view.address(),
        handshake::VA,
        "race: the control page is not at the scene's fixed address"
    );
    // 来源那条子分支盖两页：第 0 页要搬、第 1 页是目标空槽。
    let (source_at, size) = env::pole::open(source)
        .unwrap_or_else(|error| panic!("race: open source: {:?}", error.source));
    assert_eq!(
        size,
        2 * PAGE_SIZE,
        "memory-transfer-race: the source branch is two pages"
    );
    let page = Page::at(handshake::VA);
    // 把自己手里那枚**来源**子分支的号报回来：拆图探针按它认“拆的就是本景选定那一条”。
    page.publish_source_token(source);
    let va0 = source_at.get();
    let va1 = va0 + PAGE_SIZE;
    // 先读一次第 0 页：头一个字节就是那一次真缺页（物化并发布），整页按花纹核。
    let read0 = fault::check(0, va0);
    page.report(unit::self_id().get(), va0, va1, read0);
    // 等内核在在途窗口里放开：这是本景唯一那次跨 hart 放行。
    while page.start() != handshake::RELEASE {
        core::hint::spin_loop();
    }
    // 第 1 页此刻是移交留的空位：这一读就是那一次真冷缺页。
    let read1 = fault::check(0, va1);
    page.publish_read1(read1);
    // 第二段：管理端摆好请求后，交出自己那枚**来源**子分支（真入口 `Release`）。
    while page.mode() != handshake::mode::TEARDOWN {
        core::hint::spin_loop();
    }
    let released = env::pie::release(source, ReleaseMode::Revoke);
    page.publish_released(match released {
        Ok(()) => 0,
        Err(error) => error.source.code() as u64,
    });
}
