//! 共享控制页：一页私有分配，铸成一个共享根，挂在管理端空间的定点上。
//!
//! 它与迁移来源**分开**（两份来源、两个根、两个寿命）：控制页到本景收尾才关，
//! 而它两侧映射的是**同一张物理页**——管理端按 [`handshake::VA`](super::handshake::VA)
//! 读写，对端把自己那枚子分支 `open` 出来读写。
//!
//! 铸根那一手把那份引用**收下**（`create_pole` 消费它），故这里不再 release 它。

use env::AcquireSource;
use env::Allocation;
use env::PAGE_SIZE;
use env::PieToken;
use env::Placement;
use env::ReleaseMode;
use env::TaskId;
use env::TeamId;
use env::ViewControl;
use env::VirtAddr;
use execution::memory::Reference;
use execution::memory::View;

use super::source::PEER_SUBSET;
use super::source::READ_WRITE;

/// 共享控制页：一页私有分配，铸成一个共享根，挂在管理端空间的定点上。
pub struct Shared {
    view: View,
    root: PieToken,
}

impl Shared {
    /// 挂上这一页（起点就是 `handshake::VA`）并铸根。
    pub fn mint(at: usize) -> Shared {
        let allocation = Allocation::new(PAGE_SIZE, PAGE_SIZE)
            .expect("memory-transfer-race: control allocation");
        let source = AcquireSource::Allocation(allocation);
        let reference = Reference::acquire(source, PieToken::NONE)
            .unwrap_or_else(|error| panic!("race: control reference: {:?}", error.source));
        let placement = Placement::exact(VirtAddr::new(at), PAGE_SIZE)
            .expect("memory-transfer-race: control placement");
        let view = reference
            .attach(
                TeamId::new(0),
                0,
                1,
                placement,
                READ_WRITE,
                ViewControl::Caller,
            )
            .unwrap_or_else(|error| panic!("race: control view: {:?}", error.source));
        let handle = reference
            .handle()
            .expect("memory-transfer-race: control handle");
        let root = env::pole::unseal(handle, true)
        .unwrap_or_else(|error| panic!("race: control root: {:?}", error.source));
        Shared { view, root }
    }

    /// 授一枚子分支给对端：对端凭它在自己空间里把这一页 `open` 出来。
    pub fn accord(&self, task: TaskId) -> PieToken {
        env::pie::accord(self.root, task, PEER_SUBSET, env::Mark::NONE)
            .unwrap_or_else(|error| panic!("race: control accord: {:?}", error.source))
    }

    /// 收尾：先关自己那条关系，再把根交回去。
    pub fn close(&mut self) {
        self.view
            .close()
            .unwrap_or_else(|error| panic!("race: control close: {:?}", error.source));
        env::pie::release(self.root, ReleaseMode::Revoke)
            .unwrap_or_else(|error| panic!("race: control root: {:?}", error.source));
    }
}
