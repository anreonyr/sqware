#![cfg(debug_assertions)]

use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{MailCondition, Mark, PieFail, PieToken, TaskId, ToleFail};

use crate::work::mail::tole::Mate;
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
        let mut pie = gate::boxed(gate::new_pie::<gate::Hole>(meta.clone(), mark, sole, sire)).expect("pie allocation");
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

    let hole_mate = Mate::Hole(hole.id(), MailCondition::Pull);
    let bell_mate = Mate::Nole(bell.id());

    crate::expect!(
        hole_mate.key()
            == WakeKey::Hole {
                hole: hole.id().0,
                dir: MailCondition::Pull
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
    let mate = Mate::Hole(hole.id(), MailCondition::Pull);

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

/// 状态订阅那一张表：**幂等、可退、满额报 OoM 且不留半安装**；
/// 组封印要把订阅一并摘掉；没有观察者时 `signal` 一个站点都不建。
pub fn subs() {
    let target = TaskId::new(7);
    let life = Life::new();
    let weak = Arc::downgrade(&life);

    // 两格各自投一个键，且都投不出 `Tole`（订阅的转发图不可能成环）。
    crate::expect!(
        tole::Sub::TaskCompleted(target).key() == WakeKey::Task { id: target },
        "任务收尾这一类必须投 `Task` 键"
    );
    crate::expect!(
        tole::Sub::Capabilities(target).key() == WakeKey::Capabilities { task: target },
        "能力变化这一类必须投 `Capabilities` 键"
    );

    let group = tole::meta(TaskId::new(0));
    let sub = tole::Sub::TaskCompleted(target);
    tole::subscribe(&group, sub, weak.clone()).expect("第一次登记");
    crate::expect!(group.subs_len() == 1, "登记之后组里应当有一条订阅");
    tole::subscribe(&group, sub, weak.clone()).expect("重复登记");
    crate::expect!(group.subs_len() == 1, "同描述幂等：重复登记不叠加");
    tole::unsubscribe(&group, sub).expect("取消");
    crate::expect!(group.subs_len() == 0, "取消之后组里没有订阅");
    tole::unsubscribe(&group, sub).expect("重复取消");
    crate::expect!(group.subs_len() == 0, "没装过即无事：重复取消不报错");

    // 满额：`FWD_MAX` 个组都订同一个来源，第 `FWD_MAX + 1` 个报 OoM。
    let mut groups = Vec::new();
    for i in 0..FWD_MAX {
        let g = tole::meta(TaskId::new(0));
        tole::subscribe(&g, sub, weak.clone()).expect("前 FWD_MAX 个组都该订得上");
        crate::expect!(
            g.subs_len() == 1,
            "第 {} 个组应当订上（容量 {}）",
            i + 1,
            FWD_MAX
        );
        groups.push(g);
    }
    let extra = tole::meta(TaskId::new(0));
    crate::expect!(
        matches!(
            tole::subscribe(&extra, sub, weak.clone()),
            Err(ToleFail::OoM)
        ),
        "转发格满（{} 个组）时登记应当报 OoM，不静默丢",
        FWD_MAX
    );
    crate::expect!(
        extra.subs_len() == 0,
        "订不上就得把刚记的那一条退回（组里不留叫不醒的边）"
    );
    crate::expect!(
        groups.iter().all(|g| g.subs_len() == 1),
        "被拒的那一次不该动既有组的订阅"
    );

    // 封印：订阅与成员一样，得在组消亡那一趟里一起摘掉。
    for g in &groups {
        tole::seal(g);
    }
    crate::expect!(
        groups.iter().all(|g| g.subs_len() == 0),
        "封印之后订阅表应当清空"
    );
    drop(groups);

    // 含状态订阅的组**不许转授**：不借组转授绕过"只能观察自己"这一条。
    const USER_BASE: usize = 0x4000_0000;
    let space = SpaceBuilder::user().build().expect("subs: build space");
    space.with_flush(|inner| inner.dynamic(USER_BASE));
    let team = TeamBuilder::new(space).spawn().expect("subs: spawn team");
    let caller = team.task().hold().expect("subs: hold caller");
    let dst = team.task().hold().expect("subs: hold dst");
    let meta = tole::meta(caller.ident.id);
    let token = {
        let pie: gate::Pie<gate::Tole> = gate::new_pie::<gate::Tole>(
            meta.clone(),
            Mark::NONE,
            Permission::FETCH | Permission::VEST,
            None,
        );
        let token = pie.token;
        caller.gate.pies.lock().push(gate::boxed(pie).expect("pie allocation"));
        token
    };
    let dst_weak = Arc::downgrade(&dst);
    gate::accord(&caller, token, &dst_weak, Permission::FETCH, Mark::NONE)
        .expect("没装订阅时，组照旧可以转授");
    tole::subscribe(
        &meta,
        tole::Sub::Capabilities(caller.ident.id),
        caller.life(),
    )
    .expect("装上一条状态订阅");
    crate::expect!(
        matches!(
            gate::accord(&caller, token, &dst_weak, Permission::FETCH, Mark::NONE),
            Err(PieFail::Denied)
        ),
        "含状态订阅的组不许转授"
    );
    let _ = team.release_held(&caller);
    let _ = team.release_held(&dst);
    team.prune_tasks(&caller);
    team.prune_tasks(&dst);
    drop(caller);
    drop(dst);
    drop(team);
    prune_dead();

    // 没有观察者时 `signal` 不留痕：站点一个都不建、也不放行谁。
    let before = messenger::site_count();
    crate::expect!(
        messenger::signal(WakeKey::Capabilities {
            task: TaskId::new(9_999)
        }) == 0,
        "没有观察者的键上发信号应当什么都不放行"
    );
    crate::expect!(
        messenger::site_count() == before,
        "没有观察者时不为记录这一趟变化建站点"
    );
    drop(extra);
    drop(group);
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
    )).expect("pie allocation");
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
        caller.gate.pies.lock().push(gate::boxed(pie).expect("pie allocation"));
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
