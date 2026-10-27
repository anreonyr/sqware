//! 探针那一档的各台（身子在各自的 `probe_<名>/main.rs`）与它们**共用的量具**。
//!
//! 各台仍是独立 bin：由 `programs/Cargo.toml` 的 `[[bin]]` 声明、由
//! `programs::unit::catalog` 的 `PROGRAMS` 表登记它那一份 `UnitFile`。共用的东西住**本目录顶层**：
//! - [`count`]：**数一块窗格底下到齐没有**（各族"该有几枚"由各自的 `Grant::ALL.len()` 说）；
//! - [`rack`]：架那一族三台的**试验场**（那条路、那两枚砖、那一条载荷）。

pub mod count;
pub mod identity;
pub mod marks;
pub mod rack;

pub mod hierarchy;
pub mod system_fault;
pub mod device_ruin;
pub mod device_guard;
pub mod device_confirmation;
pub(crate) mod daemon_root;
pub mod daemon_rtc;
pub mod daemon_target;
pub mod daemon_stop;

pub mod memory_transfer;

// 同一条事实的另一面：`memory_transfer` 证“填回就把等待者放行”，它证“撤页之后远端
// 那颗 hart 第一次再访存必须缺页”。两台都在引导镜像那一域里，共享那族身子的量具。
pub mod memory_transfer_tlb;

// 又一面：一次真移交失败把目标还成从未物化的 Fresh。等待者与晚到者物化的都是
// 新清零页；握手页那一面与内核夹具共用 `kernel-test-api` 的同一份线格式。
pub mod memory_transfer_fresh;

// 时间那一面：上面几景都是同一颗 hart 上的先后（或只证明某一段窗口真的开着）。
// 这一景要的是**真并发**：管理端在一次真移交的在途窗口里驻留不让 hart，另一个
// Team／另一个 Space 里的对端在另一颗 hart 上真登记一次冷缺页；共享控制页那一面
// 与内核夹具共用 `kernel-test-api` 的同一份线格式。
pub mod memory_transfer_race;

// 换到硬件那一面：这台机器真的有一颗 RISC-V IOMMU。它证的是“我们写下的 DDT / DC / Sv39 表
// 真能翻”，以及拆除时 DDTP Off 与三个队列 on/busy 各自确认、同一请求随后被拒。
pub mod iommu;

pub(crate) mod fixture;

/// Run the full system acceptance image from the in-crate fixture.
pub fn acceptance() -> crate::Report<'static> {
    use crate::harness::probe::fixture::Fixture;
    use crate::system::app::{bootstrap, scene};

    let boot = bootstrap::take().expect("accept: bootstrap");
    let deployment = crate::harness::deployment(&boot.catalog).expect("accept: deployment");
    let declarations = crate::harness::declarations();
    let list = scene::programs(&boot.catalog, declarations, deployment).expect("accept: scene");
    let mut fixture = Fixture::new(boot, declarations, deployment)
        .ok()
        .expect("accept: setup");
    for program in list {
        fixture.assemble(program).expect("accept: install unit");
    }
    fixture.supervise().expect("accept: shutdown");
    crate::Report::note(env::EXIT_OK, "accept: done")
}

pub mod lifecycle;

pub mod copy;

pub mod heap;
pub mod loader;

pub mod account;
