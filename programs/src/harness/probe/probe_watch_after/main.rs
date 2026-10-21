#![no_std]
#![no_main]

//! Explicit retirement and republishing while a Watch subscriber exits.

extern crate alloc;
extern crate programs;

use core::time::Duration;

use env::Wait;
use programs::Report;

use env::unit;
use ipc::session::Session;
use programs::debug;
use system_api::operator::path::PathBuf;
use system_api::operator::{self as operator_api, Permit};
use system_client::operator;
use system_client::operator::Face;

const MS: usize = 1000;
/// 那一块窗格 —— 与 `probe-watch-gone` 订的那条路**逐字相同**（两份文件各写一遍）。
const PARENT: &str = "probe-swatch";
/// 窗格底下**那一格的名字**：本台连改的就是它（不新增子格 ⇒ 不碰 `PANE_CAP`）。
const IN: &str = "in";
/// 连改几趟（每一趟都匹配那位订户的过滤）。
const CELLS: usize = 6;
/// 两趟之间隔多久（毫秒）：跨过对面退场那一刻，同时**别挤**。
const STEP_MS: u64 = 200;
const OK_NOTE: &str = "probe-watch-after: retired and republished six times";

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(session) = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-watch-after: no tree link");
    };
    let tree = Face::from(&session);
    let service = tree
        .resolve(system_api::namespace::SERVICE, Wait::AtMost(MS))
        .unwrap();
    let road = PathBuf::try_new(PARENT).unwrap_or_else(|| panic!("probe-watch-after: bad road"));

    let client =
        system_client::operator::Capability::injected(0, unit::sire(), Wait::AtMost(MS)).unwrap();
    let anchor = client.anchor().unwrap();
    let view = operator_api::ResourceView::new(env::Access::STORE).unwrap();
    let mut binding = None;
    let mut operation = 1u64;
    for i in 0..CELLS {
        if let Some(previous) = binding.take() {
            client.unbind(previous, Wait::AtMost(MS)).unwrap();
        }
        let entry = env::hole::unseal(
            env::Mark::of("probe-swatch-after"),
            env::HoleLimits::default(),
        )
        .unwrap();
        let current = client
            .bind(
                anchor,
                IN,
                entry,
                view,
                Permit::Public,
                operator_api::OperationId(operation),
                Wait::AtMost(MS),
            )
            .unwrap()
            .binding;
        operation += 1;
        debug!(
            "probe-watch-after: republished #{i} id={}",
            current.entry_id().get()
        );
        binding = Some(current);
        // **树还答得动吗**：同一块窗格再问一次。
        let Ok(again) = tree.pane_from(service, &road, Wait::AtMost(MS)) else {
            panic!("probe-watch-after: the tree stopped answering after #{i}");
        };
        drop(again);
        let _ = execution::room::park(Duration::from_millis(STEP_MS));
    }

    let Ok(listing) = tree
        .pane_from(service, &road, Wait::AtMost(MS))
        .unwrap()
        .list(Wait::AtMost(MS))
    else {
        panic!("probe-watch-after: the tree stopped answering (list)");
    };
    let seen = listing.iter().count();
    assert_eq!(
        seen, 1,
        "那块窗格底下该只有 1 格（本台连改同一格），数到 {seen}"
    );
    Report::note(env::EXIT_OK, OK_NOTE)
}
