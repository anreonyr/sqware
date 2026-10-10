#![cfg(debug_assertions)]

use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{MailCondition, MailFail, Mark, PieFail, PieToken, TaskId};

use crate::work::mail::tole::{Cell, Member};
use crate::work::mail::{hole, nole, tole};
use crate::work::room::messenger::{self, FWD_MAX, WakeKey};
use crate::work::room::scheduler::core::prune_dead;
use crate::work::unit::gate::{self, AnyPie, Need, Permission};
use crate::work::unit::life::Life;
use crate::work::unit::space::SpaceBuilder;
use crate::work::unit::team::TeamBuilder;

pub fn form() {
    let shared = Permission::FETCH | Permission::STORE | Permission::VEST;
    let sole = shared | Permission::ONLY;

    crate::expect!(
        gate::form_ok(shared, shared),
        "共享源 ＋ 不带 ONLY 的 subset 应当一致"
    );
    crate::expect!(
        gate::form_ok(sole, sole),
        "独占源 ＋ 带 ONLY 的 subset 应当一致"
    );
    crate::expect!(
        !gate::form_ok(sole, shared),
        "想复制一枚独占资源 ⇒ 必须拒（源带 ONLY、subset 不带）"
    );
    crate::expect!(
        !gate::form_ok(shared, sole),
        "想给共享资源按上形态位 ⇒ 必须拒（源不带、subset 带）"
    );

    let mark = Mark::of("permit");
    let meta = hole::meta(TaskId::new(0));
    for sire in [None, Some(PieToken::mint(1))] {
        let mut pie = gate::boxed(gate::new_pie::<gate::Hole>(meta.clone(), mark, sole, sire))
            .expect("pie allocation");
        crate::expect!(
            gate::narrow(&mut pie, sole).is_ok(),
            "重写同一个权限集应当通过（sire = {:?}）",
            sire
        );
        crate::expect!(
            matches!(gate::narrow(&mut pie, shared), Err(PieFail::Denied)),
            "撤掉 ONLY 应当被拒——**自持枚也不例外**（sire = {:?}）",
            sire
        );
        crate::expect!(
            gate::narrow(&mut pie, Permission::FETCH | Permission::ONLY).is_ok(),
            "带着 ONLY 的普通收窄应当通过（sire = {:?}）",
            sire
        );
        crate::expect!(
            matches!(
                gate::narrow(
                    &mut pie,
                    Permission::FETCH | Permission::STORE | Permission::ONLY
                ),
                Err(PieFail::Denied)
            ),
            "非单调收窄（要一个已被收掉的位）应当被拒（sire = {:?}）",
            sire
        );
        crate::expect!(
            gate::narrow(&mut pie, Permission::ONLY).is_ok(),
            "只留 ONLY 应当通过（形态位可以独存，sire = {:?}）",
            sire
        );
    }
}

fn cell(source: env::Source, key: WakeKey, life: alloc::sync::Weak<Life>) -> Cell {
    Cell {
        source,
        actor: TaskId::new(0),
        member: Member::Mail,
        keys: Arc::new(alloc::vec![(key, life)]),
    }
}
pub fn members() {
    let group = tole::meta(TaskId::new(0));
    let h = hole::meta(TaskId::new(0));
    let source = env::Source::Mail {
        pie: PieToken::mint(123),
        condition: MailCondition::Pull,
    };
    let key = hole::key(&h, MailCondition::Pull);
    tole::attach(&group, cell(source, key, h.life())).unwrap();
    tole::attach(&group, cell(source, key, h.life())).unwrap();
    crate::expect!(group.cells_len() == 1, "重复登记幂等");
    drop(h);
    crate::expect!(group.cells_len() == 1, "失效来源保留描述以便报告和撤销");
    tole::detach(&group, TaskId::new(0), source).unwrap();
    tole::detach(&group, TaskId::new(0), source).unwrap();
    crate::expect!(group.cells_len() == 0, "失效后仍可幂等摘除");
}
pub fn fanout() {
    let h = hole::meta(TaskId::new(0));
    let key = hole::key(&h, MailCondition::Pull);
    let source = env::Source::Mail {
        pie: PieToken::mint(124),
        condition: MailCondition::Pull,
    };
    let mut groups = Vec::new();
    for _ in 0..FWD_MAX {
        let group = tole::meta(TaskId::new(0));
        tole::attach(&group, cell(source, key, h.life())).unwrap();
        groups.push(group);
    }
    let extra = tole::meta(TaskId::new(0));
    crate::expect!(
        matches!(
            tole::attach(&extra, cell(source, key, h.life())),
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
    let source = env::Source::Inspect {
        task: TaskId::new(7),
        token: PieToken::mint(1),
    };
    let group = tole::meta(TaskId::new(0));
    tole::attach(&group, cell(source, key, Arc::downgrade(&life))).unwrap();
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
    space.with_flush(|inner| inner.dynamic(USER_BASE));
    let team = TeamBuilder::new(space).spawn().expect("order: spawn team");
    let task = team.task().hold().expect("order: hold task");

    let dead_meta = hole::meta(TaskId::new(0));
    hole::seal(&dead_meta);
    let dead = gate::new_pie::<gate::Hole>(dead_meta, Mark::of("sealed"), Permission::FETCH, None);
    let dead_token = dead.token;
    let live = gate::new_pie::<gate::Hole>(
        hole::meta(TaskId::new(0)),
        Mark::of("live"),
        Permission::FETCH,
        None,
    );
    let live_token = live.token;
    {
        let mut pies = task.gate.pies.lock();
        pies.push(gate::boxed(dead).expect("pie allocation"));
        pies.push(gate::boxed(live).expect("pie allocation"));
    }

    crate::expect!(
        matches!(
            gate::accede::<PieFail>(&task, dead_token, Need::Store),
            Err(PieFail::Dead)
        ),
        "已封印 + 权不够：必须答 Dead（死活先于权限）"
    );
    crate::expect!(
        matches!(
            gate::accede(&task, live_token, Need::Store),
            Err(PieFail::Denied)
        ),
        "活着但权不够：必须答 Denied"
    );
    crate::expect!(
        gate::accede::<PieFail>(&task, live_token, Need::Fetch).is_ok(),
        "活着且权够：必须取到"
    );
    crate::expect!(
        gate::locate(&task, dead_token).is_some(),
        "locate 不过闸：已封印的那一枚也定位得到"
    );
    crate::expect!(
        matches!(
            gate::locate(&task, PieToken::mint(live_token.get() + 4_096)),
            None
        ),
        "表里没有：locate 必须答 Denied"
    );

    let released = team.release_held(&task);
    crate::expect!(released, "order: 摘出未放行任务失败");
    team.prune_tasks(&task);
    drop(task);
    drop(team);
    prune_dead();
}

pub fn badge() {
    let owner = TaskId::new(7);
    let ask = Mark::of("badge-ask");
    let reply = Mark::of("badge-reply");
    let hole = hole::meta(owner);

    let src: gate::Pie<gate::Hole> = gate::new_pie(hole.clone(), ask, Permission::FETCH, None);
    let kid: gate::Pie<gate::Hole> =
        gate::new_pie(hole.clone(), reply, Permission::FETCH, Some(src.token));
    crate::expect!(src.mark == ask, "源枚刻的是它自己那一枚");
    crate::expect!(
        kid.mark == reply,
        "子枚刻的是**自己**那一枚——同一份资源的两枚 Pie 带不同记号"
    );
    crate::expect!(src.token != kid.token, "两枚各自一号");
    crate::expect!(
        hole.owner() == owner,
        "owner 是**资源**的事实：同一份资源只有一格"
    );

    let bell_meta = nole::NoleMeta::new(owner);
    let bell: AnyPie = gate::boxed(gate::new_pie::<gate::Nole>(
        bell_meta.clone(),
        reply,
        Permission::FETCH,
        None,
    ))
    .expect("pie allocation");
    crate::expect!(bell.mark() == reply, "非孔也带记号");
    crate::expect!(
        bell.owner() == Some(owner),
        "`AnyPie::owner` 答**资源**来历（四种 Mail 都答）⇒ 孔那道闸在 `Collect` 里"
    );

    const USER_BASE: usize = 0x4000_0000;
    let space = SpaceBuilder::user().build().expect("badge: build space");
    space.with_flush(|inner| inner.dynamic(USER_BASE));
    let team = TeamBuilder::new(space).spawn().expect("badge: spawn team");
    let caller = team.task().hold().expect("badge: hold caller");
    let dst = team.task().hold().expect("badge: hold dst");

    let src_token = {
        let pie = gate::new_pie::<gate::Hole>(
            hole::meta(owner),
            ask,
            Permission::FETCH | Permission::VEST,
            None,
        );
        let token = pie.token;
        caller
            .gate
            .pies
            .lock()
            .push(gate::boxed(pie).expect("pie allocation"));
        token
    };
    let dst_weak = Arc::downgrade(&dst);
    let inherited = gate::accord(&caller, src_token, &dst_weak, Permission::FETCH, Mark::NONE)
        .expect("badge: accord (照源枚)");
    let remade = gate::accord(&caller, src_token, &dst_weak, Permission::FETCH, reply)
        .expect("badge: accord (另刻一枚)");
    crate::expect!(inherited != remade, "两次授出各落一枚");

    let kids = dst.gate.pies.lock();
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
    team.prune_tasks(&caller);
    team.prune_tasks(&dst);
    drop(caller);
    drop(dst);
    drop(team);
    prune_dead();
}

/// Exercise production Source authorization and predicates against real gates.
pub fn references() {
    use crate::runtime::switcher::envcall::tole::cell as registration;
    use env::{AwaitReply, Source, Wait};
    let space = SpaceBuilder::user().build().unwrap();
    space.with_flush(|inner| inner.dynamic(0x4000_0000));
    let team = TeamBuilder::new(space).spawn().unwrap();
    let caller = team.task().hold().unwrap();
    let target = team.task().hold().unwrap();
    let stranger = team.task().hold().unwrap();
    let group = tole::meta(caller.ident.id);
    let source_meta = hole::meta(caller.ident.id);
    let root = gate::new_pie::<gate::Hole>(
        source_meta.clone(),
        Mark::NONE,
        Permission::FETCH | Permission::STORE | Permission::VEST,
        None,
    );
    let root_token = root.token;
    caller.gate.pies.lock().push(gate::boxed(root).unwrap());
    let child = PieToken::mint(
        gate::accord(
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
    gate::reduce(&target, child, Permission::FETCH).unwrap();
    crate::expect!(
        group.poll(&caller).unwrap()
            == AwaitReply::Source {
                source,
                fail: Some(MailFail::Denied)
            },
        "丢失登记时所需访问权必须报告具体来源"
    );
    gate::release(&target, child).unwrap();
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
    // Pole bits are independent even when notifications repeat.
    let page = crate::work::mail::pole::meta(crate::memory::PAGE_SIZE, caller.ident.id).unwrap();
    let bit0 = env::Bit::FIRST;
    let bit1 = env::Bit::of(1).unwrap();
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
    // Shared groups expose only the calling registrant's exact sources.
    let local = Source::Mail {
        pie: root_token,
        condition: MailCondition::Pull,
    };
    tole::attach(&group, registration(&caller, local).unwrap()).unwrap();
    crate::expect!(
        matches!(group.poll(&stranger), Err(MailFail::Denied)),
        "共享组不转授登记者观察权"
    );
    let other_meta = hole::meta(stranger.ident.id);
    let other = gate::new_pie::<gate::Hole>(
        other_meta,
        Mark::NONE,
        Permission::FETCH | Permission::STORE,
        None,
    );
    let other_token = other.token;
    stranger.gate.pies.lock().push(gate::boxed(other).unwrap());
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
    gate::release(&stranger, other_token).unwrap();
    // Exclusive Tole authority return uses the reference edge, not a Tole-to-Tole activity edge.
    let authority = tole::meta(caller.ident.id);
    let root = gate::new_pie::<gate::Tole>(
        authority.clone(),
        Mark::NONE,
        Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
        None,
    );
    let token = root.token;
    caller.gate.pies.lock().push(gate::boxed(root).unwrap());
    let child = PieToken::mint(
        gate::accord(
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
    gate::release(&target, child).unwrap();
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
    gate::release(&caller, token).unwrap();
    gate::release(&caller, root_token).unwrap();
    for task in [&caller, &target, &stranger] {
        let _ = team.release_held(task);
        team.prune_tasks(task);
    }
    drop(caller);
    drop(target);
    drop(stranger);
    drop(team);
    prune_dead();
    let _ = Wait::POLL;
}
