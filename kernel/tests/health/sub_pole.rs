//! 精确子 Pole：从已接管的来源派生**独立子资源**，以及父侧分配槽的使用判据。
//!
//! 覆盖：子记录是自己的根（发不出私有创建资格）、重叠整体拒绝且不裁剪、不相交的
//! 父段照旧可用、嵌套时中间那一代占满自己那一段、旧引用不能绕过、失败不留半发表、
//! 正常结清后复用，以及独立保留仍存时不许复用。

#![cfg(debug_assertions)]

use alloc::sync::Arc;

use env::{MemoryCall, MemoryFail, Permission, PieToken, ReferenceRange};

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, VirtAddr as KVirt};
use crate::core::memory::nest::Access;
use crate::core::work::mail::pole;
use crate::core::work::pend;
use crate::core::work::unit::space::{Space, SpaceBuilder};
use crate::entry::call::pole::{create_pole, fail_pole_at};

#[path = "sub_pole_baseline.rs"]
mod baseline;
#[path = "sub_pole_closure.rs"]
mod closure;
#[path = "sub_pole_delivery.rs"]
mod delivery;
#[path = "sub_pole_fixture.rs"]
mod fixture;
#[path = "sub_pole_forget.rs"]
mod forget;

pub use closure::{
    parent_seal_closes_a_consumed_child, parent_seal_retries_a_failed_open,
    parent_seal_retries_a_pending_descendant, sealed_source_keeps_its_inflight_allocation,
};
pub use delivery::first_delivery_keeps_the_parent_allocation;
pub use forget::{forget_preserves_derived_indices, forget_requires_a_qualified_parent};

use baseline::Baseline;
use fixture::Fixture;

const FIRST: KVirt = KVirt::wrap(0x6300_0000);
const SECOND: KVirt = KVirt::wrap(0x6400_0000);

fn rw() -> Access {
    Access::READ | Access::WRITE
}

/// 父分支那一档的完整权限：形态跟父，故排他位也要跟。
fn whole_permission() -> Permission {
    Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY
}

fn flat_reference(handle: isize) -> env::FlatReference {
    assert!(
        handle > 0,
        "sub_pole: a handle comes from a successful call"
    );
    env::FlatReference::from_bytes(&(handle as usize as u64).to_le_bytes()).expect("reference")
}

fn token_of(raw: usize) -> PieToken {
    PieToken::from_bytes(&(raw as u64).to_le_bytes()).expect("token")
}

fn space() -> Arc<Space> {
    Arc::try_new(SpaceBuilder::user().build().unwrap()).unwrap()
}

/// 子资源是自己的根：没有私有创建资格、范围就是父资源上的那一段、
/// 父侧记的是**同一份**几何与这一条历史分支。
pub fn derived_child_is_its_own_root() {
    let baseline = Baseline::new();
    let fixture = Fixture::new(3);

    let child = fixture
        .derive(PageOffset::new(1), PageCount::new(2).unwrap(), false)
        .expect("derived child");
    assert_ne!(child, fixture.token);
    let pie = pend::locate(&fixture.task, child).expect("child Pie");
    assert!(
        pie.sire().is_none(),
        "子资源是自己的根：来源依赖不在 Pie 授权树里"
    );
    assert_eq!(pie.permission(), whole_permission(), "形态、权限跟父分支");

    let (child_meta, child_source) = fixture.child(child);
    assert_eq!(fixture.authorization.derivation_count(), 1);
    assert!(child_source.own_range().is_none(), "派生记录不持私有范围");
    assert!(
        !child_source.private_owner(fixture.task.ident.id),
        "派生记录发不出新的私有创建资格"
    );
    let window = fixture
        .source
        .range(PageOffset::new(1), PageCount::new(2).unwrap())
        .unwrap();
    assert!(
        child_meta.source().same_range(&window),
        "子段就是父资源上的那一段：坐标只换一次"
    );
    let (parent, geometry) = child_source.lineage().expect("派生记录带出生证明");
    assert!(Arc::ptr_eq(&parent, &fixture.authorization));
    assert!(
        fixture.authorization.owns_geometry(&geometry),
        "父槽与子记录共享同一份几何"
    );
    assert_eq!(
        fixture.authorization.derivation_token(&geometry),
        Some(fixture.token),
        "父槽记的是授权这一段的那一条历史分支"
    );

    // 形态与父不一致、整段父资源，都不从这里派生。
    assert_eq!(
        fixture.derive(PageOffset::ZERO, PageCount::new(1).unwrap(), true),
        Err(env::PieFail::Denied),
        "ONLY 位与父不一致就拒"
    );
    assert_eq!(
        fixture.derive(PageOffset::ZERO, PageCount::new(3).unwrap(), false),
        Err(env::PieFail::Denied),
        "整段父资源不是子段"
    );

    fixture.finish();
    // 用例自己留着的来源与关系句柄也要先放：净账量的是它们都走了之后。
    drop(pie);
    drop(child_meta);
    drop(child_source);
    drop(parent);
    drop(geometry);
    drop(window);
    baseline.expect();
}

/// 有任何重叠就整体拒绝：兄弟、整段 Open、整段 Accord 都不裁剪；
/// 不相交的父段照旧可用。
pub fn overlap_is_rejected_whole() {
    let baseline = Baseline::new();
    let fixture = Fixture::new(3);

    fixture
        .derive(PageOffset::new(1), PageCount::new(2).unwrap(), false)
        .expect("left child");
    assert_eq!(
        fixture.derive(PageOffset::new(2), PageCount::new(1).unwrap(), false),
        Err(env::PieFail::Busy),
        "兄弟重叠就拒"
    );
    assert_eq!(
        fixture.derive(PageOffset::ZERO, PageCount::new(2).unwrap(), false),
        Err(env::PieFail::Busy),
        "部分重叠也拒，不裁成剩下那一页"
    );

    let target = space();
    assert_eq!(
        pole::open(&fixture.meta, fixture.token, &target, rw()),
        Err(env::PieFail::Busy),
        "Open 操作整份资源：相交就拒"
    );
    assert_eq!(
        pend::accord_window(
            &fixture.task,
            fixture.token,
            &Arc::downgrade(&fixture.task),
            whole_permission(),
            env::Mark::NONE,
            None,
        )
        .err(),
        Some(env::PieFail::Busy),
        "Accord 同样操作整份资源"
    );

    // 不相交的一条分支窗口照旧能映出去。
    let branch = token_of(
        pend::accord_window(
            &fixture.task,
            fixture.token,
            &Arc::downgrade(&fixture.task),
            whole_permission(),
            env::Mark::NONE,
            Some((PageOffset::ZERO, PageCount::new(1).unwrap())),
        )
        .expect("disjoint accord window"),
    );
    assert!(
        pole::open(&fixture.meta, branch, &target, rw()).is_ok(),
        "不相交的父段仍可用"
    );
    fixture
        .derive(PageOffset::ZERO, PageCount::new(1).unwrap(), false)
        .expect("disjoint sibling");

    target.retire().unwrap();
    drop(target);
    fixture.finish();
    baseline.expect();
}

/// 嵌套：孙段的义务不缩小中间那一代的窗口，中间那一代仍占满自己那一段；
/// 孙段结束后中间那一代的槽才显式摘除。
pub fn nesting_holds_the_middle_window() {
    let baseline = Baseline::new();
    let fixture = Fixture::new(3);

    let middle = fixture
        .derive(PageOffset::new(1), PageCount::new(2).unwrap(), false)
        .expect("middle child");
    let (middle_meta, middle_source) = fixture.child(middle);
    let grand = fixture.reference(
        &middle_source,
        &middle_meta,
        middle,
        PageOffset::ZERO,
        PageCount::new(1).unwrap(),
    );
    let grand_token = create_pole(&fixture.task, grand, false).expect("grandchild");
    fixture.task.pend.references.remove(grand.get());
    assert_eq!(middle_source.derivation_count(), 1);
    let (_, geometry) = fixture
        .child(grand_token)
        .1
        .lineage()
        .expect("grandchild lineage");
    assert!(middle_source.owns_geometry(&geometry));

    let target = space();
    assert_eq!(
        pole::open(&fixture.meta, fixture.token, &target, rw()),
        Err(env::PieFail::Busy),
        "孙义务让中间那一代继续占满父段"
    );
    assert_eq!(
        pole::open(&middle_meta, middle, &target, rw()),
        Err(env::PieFail::Busy),
        "中间那一代自己不空"
    );

    pend::release(&fixture.task, grand_token).expect("release grandchild");
    assert_eq!(
        middle_source.derivation_count(),
        0,
        "孙段结束即摘中间那一代的槽"
    );
    assert_eq!(
        fixture.authorization.derivation_count(),
        1,
        "中间那一代自己仍占着父段"
    );

    target.retire().unwrap();
    drop(target);
    fixture.finish();
    drop(middle_meta);
    drop(middle_source);
    baseline.expect();
}

/// 失败不留半发表：父槽、Pie 与父记录都不动，同一段还能再来一次。
pub fn failed_derivation_publishes_nothing() {
    let baseline = Baseline::new();
    let fixture = Fixture::new(3);
    let pies = fixture.task.pend.pies.lock().len();

    fail_pole_at(6);
    assert_eq!(
        fixture.derive(PageOffset::new(1), PageCount::new(2).unwrap(), false),
        Err(env::PieFail::OoM)
    );
    assert_eq!(fixture.authorization.derivation_count(), 0, "失败不留父槽");
    assert_eq!(fixture.task.pend.pies.lock().len(), pies, "失败不留 Pie");

    let retry = fixture
        .derive(PageOffset::new(1), PageCount::new(2).unwrap(), false)
        .expect("retry after a failed preparation");
    assert_ne!(retry, fixture.token);
    assert_eq!(fixture.authorization.derivation_count(), 1);

    fixture.finish();
    baseline.expect();
}

/// 正常结清后父段复用；独立持有仍存时不许复用（包括建新授权），
/// 持有结束的显式事件才把这一段交回来。
pub fn reuse_waits_for_settlement_and_held_frames() {
    let baseline = Baseline::new();
    let fixture = Fixture::new(3);

    // 先分出一段子资源，再要一份独立持有：子段仍占着时这一份持有被拒。
    let child = fixture
        .derive(PageOffset::new(1), PageCount::new(1).unwrap(), false)
        .expect("child");
    let cap = crate::entry::boot::resources::grant_retention(&fixture.task).expect("retention");
    assert_eq!(
        fixture.acquire(fixture.token, 1, 1, cap.token()),
        MemoryFail::Busy.code(),
        "子段仍占着那一段：独立持有不建"
    );
    assert_eq!(fixture.authorization.retention_count(), 0);

    // 子资源正常结清（显式摘掉最后一枚授权）：槽随之收掉，这一段不再被占用。
    pend::release(&fixture.task, child).expect("release child");
    assert_eq!(fixture.authorization.derivation_count(), 0);

    // 持有人自己那一段映得出去；同一段也能建成独立持有。
    let retained = fixture.acquire(fixture.token, 1, 1, cap.token());
    assert!(retained > 0, "结清后这一段可以独立持有");
    assert_eq!(fixture.authorization.retention_count(), 1);
    assert!(
        fixture.attach(retained, 0, 1, SECOND).0 > 0,
        "持有人自己那段映得出去"
    );

    // 独立持有仍在：这一段不许建新授权（派生 / Open 都不行），
    // 不相交的父段照旧可用。
    assert_eq!(
        fixture.derive(PageOffset::new(1), PageCount::new(1).unwrap(), false),
        Err(env::PieFail::Busy),
        "独立持有仍在：这一段不许派生"
    );
    let target = space();
    assert_eq!(
        pole::open(&fixture.meta, fixture.token, &target, rw()),
        Err(env::PieFail::Busy),
        "整段 Open 与它相交就拒"
    );
    assert!(
        fixture
            .derive(PageOffset::ZERO, PageCount::new(1).unwrap(), false)
            .is_ok(),
        "不相交的父段照旧可用"
    );

    // 独立持有解除是显式的结清事件：这一段回来。
    assert_eq!(
        fixture
            .call(MemoryCall::Release {
                reference: flat_reference(retained),
            })
            .0,
        0,
        "独立持有解除"
    );
    assert_eq!(fixture.authorization.retention_count(), 0);
    let again = fixture
        .derive(PageOffset::new(1), PageCount::new(1).unwrap(), false)
        .expect("真正结束之后这一段又能派生");
    assert_ne!(again, fixture.token);
    assert_eq!(fixture.authorization.derivation_count(), 2);

    target.retire().unwrap();
    drop(target);
    fixture.finish();
    baseline.expect();
}

/// Seal 之后死 token 不永久占位：子记录不再有可用授权时，
/// 父侧下一次判占用就把这一槽收掉，父段回来。
pub fn sealed_child_does_not_hold_its_segment() {
    let baseline = Baseline::new();
    let fixture = Fixture::new(3);

    let child = fixture
        .derive(PageOffset::new(1), PageCount::new(2).unwrap(), false)
        .expect("child");
    let (child_meta, _) = fixture.child(child);
    crate::core::work::mail::pole::seal(&child_meta);
    assert!(!child_meta.authorizing(), "Seal 之后不再有可用授权");

    let target = space();
    assert!(
        pole::open(&fixture.meta, fixture.token, &target, rw()).is_ok(),
        "死 token 不永久占位：父段回来"
    );
    assert_eq!(fixture.authorization.derivation_count(), 0);

    target.retire().unwrap();
    drop(target);
    drop(child_meta);
    fixture.finish();
    baseline.expect();
}

/// 旧引用绕行：子段分出**之前**取得的整段引用，也不能把子段重新映出去；
/// 同一份旧引用里不相交的那一段照旧可用。
pub fn old_reference_cannot_bypass_the_gate() {
    let baseline = Baseline::new();
    let fixture = Fixture::new(3);

    let old = fixture.acquire(fixture.token, 0, 3, PieToken::NONE);
    assert!(old > 0, "旧引用取得");
    fixture
        .derive(PageOffset::new(1), PageCount::new(2).unwrap(), false)
        .expect("child");

    assert_eq!(
        fixture.attach(old, 0, 3, SECOND).0,
        MemoryFail::Busy.code(),
        "旧引用要映整段：与子段相交就整体拒绝"
    );

    let sub = fixture
        .call(MemoryCall::Acquire {
            source: env::AcquireSource::Reference(
                ReferenceRange::new(flat_reference(old), 0, 1).expect("reference range"),
            ),
            retention: PieToken::NONE,
        })
        .0;
    assert!(sub > 0, "同一份旧引用里不相交的子窗口仍可取得");
    let (view, address) = fixture.attach(sub, 0, 1, SECOND);
    assert!(view > 0, "不相交的父段仍能映出去");
    assert_eq!(address, SECOND.as_usize());

    for handle in [sub, old] {
        assert_eq!(
            fixture
                .call(MemoryCall::Release {
                    reference: flat_reference(handle),
                })
                .0,
            0
        );
    }

    fixture.finish();
    baseline.expect();
}

pub fn retained_child_blocks_parent_reuse() {
    for seal in [false, true] {
        let baseline = Baseline::new();
        let fixture = Fixture::new(3);
        let child = fixture
            .derive(PageOffset::new(1), PageCount::new(1).unwrap(), false)
            .expect("child");
        let (child_meta, child_source) = fixture.child(child);
        let capability =
            crate::entry::boot::resources::grant_retention(&fixture.task).expect("retention");
        let retained = fixture.acquire(child, 0, 1, capability.token());
        assert!(retained > 0, "retain the child resource, not its parent");
        assert_eq!(child_source.retention_count(), 1);

        if seal {
            crate::core::work::mail::pole::seal(&child_meta);
        } else {
            pend::release(&fixture.task, child).expect("release the child's last authorization");
        }
        assert!(!child_meta.authorizing());
        assert_eq!(
            fixture.derive(PageOffset::new(1), PageCount::new(1).unwrap(), false),
            Err(env::PieFail::Busy),
            "a retained child must keep its parent's allocation occupied"
        );
        assert_eq!(fixture.authorization.derivation_count(), 1);

        assert_eq!(
            fixture
                .call(MemoryCall::Release {
                    reference: flat_reference(retained),
                })
                .0,
            0
        );
        assert_eq!(child_source.retention_count(), 0);
        assert_eq!(
            fixture.authorization.derivation_count(),
            0,
            "releasing the last retention must settle the parent slot immediately"
        );
        fixture
            .derive(PageOffset::new(1), PageCount::new(1).unwrap(), false)
            .expect("reuse only after the child's retention has ended");

        drop(child_meta);
        drop(child_source);
        fixture.finish();
        baseline.expect();
    }
}

pub fn old_reference_cannot_transfer_a_child_page() {
    let baseline = Baseline::new();
    let fixture = Fixture::new(3);
    let old = fixture.acquire(fixture.token, 0, 3, PieToken::NONE);
    let target = fixture
        .call(MemoryCall::Acquire {
            source: env::AcquireSource::Allocation(
                env::Allocation::new(PAGE_SIZE, PAGE_SIZE).expect("allocation"),
            ),
            retention: PieToken::NONE,
        })
        .0;
    assert!(old > 0 && target > 0);
    let child = fixture
        .derive(PageOffset::new(1), PageCount::new(1).unwrap(), false)
        .expect("child");
    let address = KVirt::wrap(FIRST.as_usize() + PAGE_SIZE);
    let physical = fixture
        .team
        .space
        .nest
        .translate(address)
        .map(|(address, _)| address.as_usize());
    assert!(physical.is_some());
    let transfer = || MemoryCall::Transfer {
        from: flat_reference(old),
        from_page: 1,
        to: flat_reference(target),
        to_page: 0,
    };
    assert_eq!(
        fixture.call(transfer()).0,
        MemoryFail::Busy.code(),
        "an old parent reference cannot move a derived child's page"
    );
    assert_eq!(
        fixture
            .team
            .space
            .nest
            .translate(address)
            .map(|(address, _)| address.as_usize()),
        physical
    );

    pend::release(&fixture.task, child).expect("settle the child");
    assert_eq!(
        fixture.call(transfer()).0,
        0,
        "the same transfer becomes valid after the child has settled"
    );
    assert!(fixture.team.space.nest.translate(address).is_none());
    fixture.finish();
    baseline.expect();
}

pub fn publication_keys_keep_their_domains() {
    let baseline = Baseline::new();
    let fixture = Fixture::new(3);
    let mapping = 17;
    let flight = (1usize << (usize::BITS - 1)) | mapping;
    let count = PageCount::new(1).unwrap();
    let authorization = &fixture.authorization;
    {
        let _commit = crate::core::work::commit();
        authorization
            .enter_publish(mapping, None, Access::READ)
            .unwrap();
        for offset in [PageOffset::ZERO, PageOffset::new(1)] {
            authorization
                .enter_publish_range(
                    flight,
                    pend::Geometry::new(offset, count).expect("flight window"),
                )
                .unwrap();
        }

        authorization.leave_publish(mapping);
        assert!(matches!(
            authorization.begin_withdraw(),
            Err(env::PieFail::Busy)
        ));
        for offset in [PageOffset::ZERO, PageOffset::new(1)] {
            assert_eq!(
                authorization.check_publish(offset, count, None),
                Err(env::PieFail::Busy),
                "mapping cleanup must leave both transfer windows registered"
            );
        }

        authorization
            .enter_publish(mapping, None, Access::READ)
            .unwrap();
        authorization.leave_publish(flight);
        assert!(matches!(
            authorization.begin_withdraw(),
            Err(env::PieFail::Busy)
        ));
        for offset in [PageOffset::ZERO, PageOffset::new(1)] {
            assert_eq!(authorization.check_publish(offset, count, None), Ok(()));
        }
        authorization.leave_publish(mapping);
        assert!(authorization.begin_withdraw().is_ok());
    }
    fixture.finish();
    baseline.expect();
}
