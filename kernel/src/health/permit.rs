#![cfg(debug_assertions)]

use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{MailCondition, MailFail, Mark, PieFail, PieToken, TaskId};

use crate::work::mail::Mail;
use crate::work::mail::tole::{Cell, Watch};
use crate::work::mail::{hole, nole, tole};
use crate::work::pend::{self, Need, Permission, Pie};
use crate::work::room::messenger::{self, FWD_MAX, WakeKey};
use crate::work::room::scheduler::core::prune_dead;
use crate::work::unit::life::Life;
use crate::work::unit::space::SpaceBuilder;
use crate::work::unit::team::TeamBuilder;

pub fn form() {
    let shared = Permission::FETCH | Permission::STORE | Permission::VEST;
    let sole = shared | Permission::ONLY;

    crate::expect!(
        pend::form_ok(shared, shared),
        "共享源 ＋ 不带 ONLY 的 subset 应当一致"
    );
    crate::expect!(
        pend::form_ok(sole, sole),
        "独占源 ＋ 带 ONLY 的 subset 应当一致"
    );
    crate::expect!(
        !pend::form_ok(sole, shared),
        "想复制一枚独占资源 ⇒ 必须拒（源带 ONLY、subset 不带）"
    );
    crate::expect!(
        !pend::form_ok(shared, sole),
        "想给共享资源按上形态位 ⇒ 必须拒（源不带、subset 带）"
    );

    let mark = Mark::of("permit");
    let meta = hole::HoleMeta::new(TaskId::new(0));
    for sire in [None, Some(PieToken::mint(1))] {
        let mut pie =
            pend::Pie::try_new(Mail::Hole(meta.clone()), mark, sole, sire).expect("pie allocation");
        crate::expect!(
            pie.narrow(sole).is_ok(),
            "重写同一个权限集应当通过（sire = {:?}）",
            sire
        );
        crate::expect!(
            matches!(pie.narrow(shared), Err(PieFail::Denied)),
            "撤掉 ONLY 应当被拒——**自持枚也不例外**（sire = {:?}）",
            sire
        );
        crate::expect!(
            pie.narrow(Permission::FETCH | Permission::ONLY).is_ok(),
            "带着 ONLY 的普通收窄应当通过（sire = {:?}）",
            sire
        );
        crate::expect!(
            matches!(
                pie.narrow(Permission::FETCH | Permission::STORE | Permission::ONLY),
                Err(PieFail::Denied)
            ),
            "非单调收窄（要一个已被收掉的位）应当被拒（sire = {:?}）",
            sire
        );
        crate::expect!(
            pie.narrow(Permission::ONLY).is_ok(),
            "只留 ONLY 应当通过（形态位可以独存，sire = {:?}）",
            sire
        );
    }
}

fn cell(watch: Watch, key: WakeKey, life: alloc::sync::Weak<Life>) -> Cell {
    Cell {
        actor: TaskId::new(0),
        keys: Arc::new(alloc::vec![(key, life)]),
        watch,
    }
}
pub fn members() {
    let group = tole::ToleMeta::new(TaskId::new(0));
    let h = hole::HoleMeta::new(TaskId::new(0));
    let watch = Watch::Mail {
        pie: PieToken::mint(123),
        condition: MailCondition::Pull,
    };
    let key = hole::key(&h, MailCondition::Pull);
    tole::attach(&group, cell(watch.clone(), key, h.life())).unwrap();
    tole::attach(&group, cell(watch.clone(), key, h.life())).unwrap();
    crate::expect!(group.cells_len() == 1, "重复登记幂等");
    drop(h);
    crate::expect!(group.cells_len() == 1, "失效来源保留描述以便报告和撤销");
    tole::detach(&group, TaskId::new(0), watch.source()).unwrap();
    tole::detach(&group, TaskId::new(0), watch.source()).unwrap();
    crate::expect!(group.cells_len() == 0, "失效后仍可幂等摘除");
}
pub fn fanout() {
    let h = hole::HoleMeta::new(TaskId::new(0));
    let key = hole::key(&h, MailCondition::Pull);
    let watch = Watch::Mail {
        pie: PieToken::mint(124),
        condition: MailCondition::Pull,
    };
    let mut groups = Vec::new();
    for _ in 0..FWD_MAX {
        let group = tole::ToleMeta::new(TaskId::new(0));
        tole::attach(&group, cell(watch.clone(), key, h.life())).unwrap();
        groups.push(group);
    }
    let extra = tole::ToleMeta::new(TaskId::new(0));
    crate::expect!(
        matches!(
            tole::attach(&extra, cell(watch.clone(), key, h.life())),
            Err(MailFail::OoM)
        ),
        "转发满额显式失败"
    );
    crate::expect!(extra.cells_len() == 0, "失败完整回滚");
    crate::expect!(
        groups.iter().all(|g| g.cells_len() == 1),
        "失败不影响既有登记"
    );
}
pub fn subs() {
    let life = Life::new();
    let key = WakeKey::Inspect {
        task: TaskId::new(7),
        token: 1,
    };
    let watch = Watch::Inspect {
        task: TaskId::new(7),
        token: PieToken::mint(1),
        holder: alloc::sync::Weak::new(),
        parent: None,
        held: false,
        access: env::Permission::empty(),
    };
    let group = tole::ToleMeta::new(TaskId::new(0));
    tole::attach(&group, cell(watch, key, Arc::downgrade(&life))).unwrap();
    crate::expect!(group.has_cells(), "登记身份阻止组转授观察权");
    tole::seal(&group);
    crate::expect!(group.cells_len() == 0, "封印清理全部登记");
    let before = messenger::site_count();
    crate::expect!(
        messenger::signal(WakeKey::Inspect {
            task: TaskId::new(9999),
            token: 9
        }) == 0,
        "未登记的引用变化不建站点"
    );
    crate::expect!(messenger::site_count() == before, "没有引用观察者不留痕");
}

pub fn order() {
    const USER_BASE: usize = 0x4000_0000;
    let space = SpaceBuilder::user().build().expect("order: build space");
    space
        .attach_region(
            crate::work::unit::space::RegionId::Normal,
            USER_BASE,
            crate::memory::manager::mode::upper().as_usize(),
        )
        .unwrap();
    let team = TeamBuilder::new(space).spawn().expect("order: spawn team");
    let task = team.task().hold().expect("order: hold task");

    let dead_meta = hole::HoleMeta::new(TaskId::new(0));
    hole::seal(&dead_meta);
    let dead = pend::Pie::try_new(
        Mail::Hole(dead_meta),
        Mark::of("sealed"),
        Permission::FETCH,
        None,
    )
    .expect("pie allocation");
    let dead_token = dead.token();
    let live = pend::Pie::try_new(
        Mail::Hole(hole::HoleMeta::new(TaskId::new(0))),
        Mark::of("live"),
        Permission::FETCH,
        None,
    )
    .expect("pie allocation");
    let live_token = live.token();
    {
        let mut pies = task.pend.pies.lock();
        pies.push(dead);
        pies.push(live);
    }

    crate::expect!(
        matches!(
            pend::accede(&task, dead_token, Need::Store),
            Err(PieFail::Dead)
        ),
        "已封印 + 权不够：必须答 Dead（死活先于权限）"
    );
    crate::expect!(
        matches!(
            pend::accede(&task, live_token, Need::Store),
            Err(PieFail::Denied)
        ),
        "活着但权不够：必须答 Denied"
    );
    crate::expect!(
        pend::accede(&task, live_token, Need::Fetch).is_ok(),
        "活着且权够：必须取到"
    );
    crate::expect!(
        pend::locate(&task, dead_token).is_some(),
        "locate 不过闸：已封印的那一枚也定位得到"
    );
    crate::expect!(
        matches!(
            pend::locate(&task, PieToken::mint(live_token.get() + 4_096)),
            None
        ),
        "表里没有：locate 必须答 Denied"
    );

    let released = team.release_held(&task);
    crate::expect!(released, "order: 摘出未放行任务失败");
    team.prune_tasks();
    drop(task);
    drop(team);
    prune_dead();
}

pub fn badge() {
    let owner = TaskId::new(7);
    let ask = Mark::of("badge-ask");
    let reply = Mark::of("badge-reply");
    let hole = hole::HoleMeta::new(owner);

    let src = pend::Pie::try_new(Mail::Hole(hole.clone()), ask, Permission::FETCH, None)
        .expect("pie allocation");
    let kid = pend::Pie::try_new(
        Mail::Hole(hole.clone()),
        reply,
        Permission::FETCH,
        Some(src.token()),
    )
    .expect("pie allocation");
    crate::expect!(src.mark() == ask, "源枚刻的是它自己那一枚");
    crate::expect!(
        kid.mark() == reply,
        "子枚刻的是**自己**那一枚——同一份资源的两枚 Pie 带不同记号"
    );
    crate::expect!(src.token() != kid.token(), "两枚各自一号");
    crate::expect!(
        hole.owner() == owner,
        "owner 是**资源**的事实：同一份资源只有一格"
    );

    let bell_meta = nole::NoleMeta::new(owner);
    let bell: Pie = pend::Pie::try_new(
        Mail::Nole(bell_meta.clone()),
        reply,
        Permission::FETCH,
        None,
    )
    .expect("pie allocation");
    crate::expect!(bell.mark() == reply, "非孔也带记号");
    crate::expect!(
        bell.owner() == Some(owner),
        "`Pie` 的 owner 答**资源**来历（四种 Mail 都答）⇒ 孔那道闸在 `Collect` 里"
    );

    const USER_BASE: usize = 0x4000_0000;
    let space = SpaceBuilder::user().build().expect("badge: build space");
    space
        .attach_region(
            crate::work::unit::space::RegionId::Normal,
            USER_BASE,
            crate::memory::manager::mode::upper().as_usize(),
        )
        .unwrap();
    let team = TeamBuilder::new(space).spawn().expect("badge: spawn team");
    let caller = team.task().hold().expect("badge: hold caller");
    let dst = team.task().hold().expect("badge: hold dst");

    let src_token = {
        let pie = pend::Pie::try_new(
            Mail::Hole(hole::HoleMeta::new(owner)),
            ask,
            Permission::FETCH | Permission::VEST,
            None,
        )
        .expect("pie allocation");
        let token = pie.token();
        caller.pend.pies.lock().push(pie);
        token
    };
    let dst_weak = Arc::downgrade(&dst);
    let inherited = pend::accord(&caller, src_token, &dst_weak, Permission::FETCH, Mark::NONE)
        .expect("badge: accord (照源枚)");
    let remade = pend::accord(&caller, src_token, &dst_weak, Permission::FETCH, reply)
        .expect("badge: accord (另刻一枚)");
    crate::expect!(inherited != remade, "两次授出各落一枚");

    let kids = dst.pend.pies.lock();
    let mark_of = |token: usize| {
        kids.iter()
            .find(|p| p.token().get() == token)
            .map(|p| p.mark())
    };
    crate::expect!(
        mark_of(inherited) == Some(ask),
        "`Accord` 不给记号（`NONE`）⇒ 子枚照源枚"
    );
    crate::expect!(
        mark_of(remade) == Some(reply),
        "`Accord` 给了记号 ⇒ 子枚刻那一枚"
    );
    drop(kids);

    let _ = team.release_held(&caller);
    let _ = team.release_held(&dst);
    team.prune_tasks();
    team.prune_tasks();
    drop(caller);
    drop(dst);
    drop(team);
    prune_dead();
}

pub fn references() {
    use crate::runtime::switcher::envcall::tole::cell as registration;
    use env::{AwaitReply, Source, Wait};
    let space = SpaceBuilder::user().build().unwrap();
    space
        .attach_region(
            crate::work::unit::space::RegionId::Normal,
            0x4000_0000,
            crate::memory::manager::mode::upper().as_usize(),
        )
        .unwrap();
    let team = TeamBuilder::new(space).spawn().unwrap();
    let caller = team.task().hold().unwrap();
    let target = team.task().hold().unwrap();
    let stranger = team.task().hold().unwrap();
    let group = tole::ToleMeta::new(caller.ident.id);
    let source_meta = hole::HoleMeta::new(caller.ident.id);
    let root = pend::Pie::try_new(
        Mail::Hole(source_meta.clone()),
        Mark::NONE,
        Permission::FETCH | Permission::STORE | Permission::VEST,
        None,
    )
    .expect("pie allocation");
    let root_token = root.token();
    caller.pend.pies.lock().push(root);
    let child = PieToken::mint(
        pend::accord(
            &caller,
            root_token,
            &Arc::downgrade(&target),
            Permission::FETCH | Permission::STORE,
            Mark::NONE,
        )
        .unwrap(),
    );
    let source = Source::Inspect {
        task: target.ident.id,
        token: child,
    };
    crate::expect!(
        matches!(registration(&stranger, source), Err(MailFail::Denied)),
        "不能靠知道目标编号观察无关引用"
    );
    tole::attach(&group, registration(&caller, source).unwrap()).unwrap();
    crate::expect!(
        group.poll(&caller).unwrap() == AwaitReply::Pending,
        "仍有效的端点不提前关闭"
    );
    pend::reduce(&target, child, Permission::FETCH).unwrap();
    crate::expect!(
        group.poll(&caller).unwrap()
            == AwaitReply::Source {
                source,
                fail: Some(MailFail::Denied)
            },
        "丢失登记时所需访问权必须报告具体来源"
    );
    pend::release(&target, child).unwrap();
    crate::expect!(
        group.poll(&caller).unwrap()
            == AwaitReply::Source {
                source,
                fail: Some(MailFail::Denied)
            },
        "主动 Release 被检测，任务和资源仍存活"
    );
    tole::detach(&group, caller.ident.id, source).unwrap();
    crate::expect!(
        group.poll(&caller).unwrap() == AwaitReply::Pending,
        "失效后可摘除原描述"
    );
    let reservation = caller
        .ident
        .team
        .space
        .allocate_low(
            crate::work::unit::space::RegionId::Normal,
            crate::memory::PAGE_SIZE,
        )
        .unwrap();
    let frame = crate::memory::manager::Frame::allocate().unwrap();
    let rw =
        crate::memory::manager::entry::PteFlags::R | crate::memory::manager::entry::PteFlags::W;
    let mapping = caller
        .ident
        .team
        .space
        .map_frames(reservation, alloc::vec![frame], rw, rw)
        .unwrap();
    let page = crate::work::mail::pole::PoleMeta::from_mapping(
        &caller.ident.team.space,
        mapping,
        caller.ident.id,
    )
    .unwrap();
    page.commit_source();
    let bit0 = env::Bit::FIRST;
    let bit1 = env::Bit::wrap(1);
    crate::work::mail::pole::ring(&page, bit0).unwrap();
    crate::work::mail::pole::ring(&page, bit0).unwrap();
    crate::work::mail::pole::ring(&page, bit1).unwrap();
    crate::work::mail::pole::hush(&page, bit0).unwrap();
    crate::work::mail::pole::hush(&page, bit0).unwrap();
    crate::expect!(
        !page.ready(bit0) && page.ready(bit1),
        "重复清位幂等且不影响另一位"
    );
    drop(page);
    let local = Source::Mail {
        pie: root_token,
        condition: MailCondition::Pull,
    };
    tole::attach(&group, registration(&caller, local).unwrap()).unwrap();
    crate::expect!(
        matches!(group.poll(&stranger), Err(MailFail::Denied)),
        "共享组不转授登记者观察权"
    );
    let other_meta = hole::HoleMeta::new(stranger.ident.id);
    let other = pend::Pie::try_new(
        Mail::Hole(other_meta),
        Mark::NONE,
        Permission::FETCH | Permission::STORE,
        None,
    )
    .expect("pie allocation");
    let other_token = other.token();
    stranger.pend.pies.lock().push(other);
    let other_source = Source::Mail {
        pie: other_token,
        condition: MailCondition::Pull,
    };
    tole::attach(&group, registration(&stranger, other_source).unwrap()).unwrap();
    crate::expect!(
        group.poll(&caller).unwrap() == AwaitReply::Pending
            && group.poll(&stranger).unwrap() == AwaitReply::Pending,
        "各登记者分别复核自己的来源"
    );
    tole::detach(&group, stranger.ident.id, other_source).unwrap();
    tole::detach(&group, caller.ident.id, local).unwrap();
    pend::release(&stranger, other_token).unwrap();
    let authority = tole::ToleMeta::new(caller.ident.id);
    let root = pend::Pie::try_new(
        Mail::Tole(authority.clone()),
        Mark::NONE,
        Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
        None,
    )
    .expect("pie allocation");
    let token = root.token();
    caller.pend.pies.lock().push(root);
    let child = PieToken::mint(
        pend::accord(
            &caller,
            token,
            &Arc::downgrade(&target),
            Permission::FETCH | Permission::ONLY,
            Mark::NONE,
        )
        .unwrap(),
    );
    let source = Source::Inspect {
        task: caller.ident.id,
        token,
    };
    tole::attach(&group, registration(&caller, source).unwrap()).unwrap();
    crate::expect!(
        group.poll(&caller).unwrap() == AwaitReply::Pending,
        "独占移交期间等待归还，不能持续报告 HandedOver"
    );
    pend::release(&target, child).unwrap();
    crate::expect!(
        group.poll(&caller).unwrap() == AwaitReply::Source { source, fail: None },
        "子引用释放后祖先恢复使用权"
    );
    tole::seal(&authority);
    crate::expect!(
        group.poll(&caller).unwrap()
            == AwaitReply::Source {
                source,
                fail: Some(MailFail::Dead)
            },
        "封印和归还具有不同的来源状态"
    );
    tole::detach(&group, caller.ident.id, source).unwrap();
    tole::seal(&group);
    pend::release(&caller, token).unwrap();
    pend::release(&caller, root_token).unwrap();
    for task in [&caller, &target, &stranger] {
        let _ = team.release_held(task);
        team.prune_tasks();
    }
    drop(caller);
    drop(target);
    drop(stranger);
    drop(team);
    prune_dead();
    let _ = Wait::POLL;
}
