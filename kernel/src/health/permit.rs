#![cfg(debug_assertions)]

use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{HoleDir, Mark, PieFail, PieToken, TaskId, ToleFail};

use crate::work::mail::tole::Mate;
use crate::work::mail::{hole, nole, tole};
use crate::work::room::messenger::{FWD_MAX, WakeKey};
use crate::work::room::scheduler::core::prune_dead;
use crate::work::unit::gate::{self, AnyPie, Need, Permission};
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
        let mut pie = AnyPie::Hole(gate::new_pie(meta.clone(), mark, sole, sire));
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

pub fn members() {
    let group = tole::meta(TaskId::new(0));
    let hole = hole::meta(TaskId::new(0));
    let bell = nole::NoleMeta::new(TaskId::new(0));

    let hole_mate = Mate::Hole(hole.id(), HoleDir::Pull);
    let bell_mate = Mate::Nole(bell.id());

    crate::expect!(
        hole_mate.key()
            == WakeKey::Hole {
                hole: hole.id().0,
                dir: HoleDir::Pull
            },
        "孔的格子必须投影成 Hole 键"
    );
    crate::expect!(
        bell_mate.key() == WakeKey::Nole { id: bell.id().0 },
        "铃的格子必须投影成 Nole 键"
    );

    tole::attach(&group, hole_mate, hole.life()).expect("挂孔");
    crate::expect!(group.cells().len() == 1, "挂一格之后快照应当有一格");
    tole::attach(&group, hole_mate, hole.life()).expect("重复挂孔");
    crate::expect!(group.cells().len() == 1, "同成员幂等：重复挂不叠加");

    tole::attach(&group, bell_mate, bell.life()).expect("挂铃");
    crate::expect!(
        group.cells().len() == 2,
        "两种成员各占一格（孔的另一个方向可再占一格）"
    );

    tole::detach(&group, hole_mate).expect("摘孔");
    crate::expect!(group.cells().len() == 1, "摘掉一格后只剩铃那一格");
    tole::detach(&group, hole_mate).expect("重复摘孔");
    crate::expect!(
        group.cells().len() == 1,
        "没挂过即无事：重复摘不报错也不多加"
    );

    drop(bell);
    crate::expect!(
        group.cells().is_empty(),
        "铃没了之后，它那一格不该再出现在快照里"
    );
}

pub fn fanout() {
    let hole = hole::meta(TaskId::new(0));
    let mate = Mate::Hole(hole.id(), HoleDir::Pull);

    let mut groups = Vec::new();
    for i in 0..FWD_MAX {
        let group = tole::meta(TaskId::new(0));
        tole::attach(&group, mate, hole.life()).expect("前 FWD_MAX 个组都该挂得上");
        crate::expect!(
            group.cells().len() == 1,
            "第 {} 个组应当挂上（容量 {}）",
            i + 1,
            FWD_MAX
        );
        groups.push(group);
    }

    let extra = tole::meta(TaskId::new(0));
    crate::expect!(
        matches!(tole::attach(&extra, mate, hole.life()), Err(ToleFail::OoM)),
        "转发格满（{} 个组）时挂格应当报 OoM，不静默丢",
        FWD_MAX
    );
    crate::expect!(
        extra.cells().is_empty(),
        "挂不上就得把刚挂的那一格退回（池里不留叫不醒的格子）"
    );
    crate::expect!(
        groups.iter().all(|g| g.cells().len() == 1),
        "被拒的那一次不该动既有组的格子"
    );
}

pub fn order() {
    const USER_BASE: usize = 0x4000_0000;
    let space = SpaceBuilder::user().build().expect("order: build space");
    space.with_flush(|inner| inner.dynamic(USER_BASE));
    let team = TeamBuilder::new(space).spawn().expect("order: spawn team");
    let task = team.task().hold().expect("order: hold task");

    let dead_meta = hole::meta(TaskId::new(0));
    hole::seal(&dead_meta);
    let dead = gate::new_pie(dead_meta, Mark::of("sealed"), Permission::FETCH, None);
    let dead_token = dead.token;
    let live = gate::new_pie(
        hole::meta(TaskId::new(0)),
        Mark::of("live"),
        Permission::FETCH,
        None,
    );
    let live_token = live.token;
    {
        let mut pies = task.pies.lock();
        pies.push(AnyPie::Hole(dead));
        pies.push(AnyPie::Hole(live));
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
    let bell: AnyPie = AnyPie::Nole(gate::new_pie(
        bell_meta.clone(),
        reply,
        Permission::FETCH,
        None,
    ));
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
        let pie = gate::new_pie(
            hole::meta(owner),
            ask,
            Permission::FETCH | Permission::VEST,
            None,
        );
        let token = pie.token;
        caller.pies.lock().push(AnyPie::Hole(pie));
        token
    };
    let dst_weak = Arc::downgrade(&dst);
    let inherited = gate::accord(&caller, src_token, &dst_weak, Permission::FETCH, Mark::NONE)
        .expect("badge: accord (照源枚)");
    let remade = gate::accord(&caller, src_token, &dst_weak, Permission::FETCH, reply)
        .expect("badge: accord (另刻一枚)");
    crate::expect!(inherited != remade, "两次授出各落一枚");

    let kids = dst.pies.lock();
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