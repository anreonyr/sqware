// 健康检查 · permit —— 权柄代数的**形态位**（`ONLY`）、组的**成员投影**、
// **转发容量**与**取用顺序**。
//
// 四件事在这里被证（都只经公开接口，不碰内部字段）：
//
//   · **形态位不是选择，是一致性**（`gate::form_ok`）+ **`ONLY` 不可撤**
//     （`gate::narrow`，**自持枚也不例外**）——这两条是"独占资源复制不出去"的两条腿；
//     缺任一条，`ONLY` 就退回成一句空声明（旧 `CAGE` 的读法就是这么退化的）。
//   · **组的成员投影**：两种成员（孔 / 铃）都挂得上、同成员幂等、快照按存活过滤，
//     且各自的等待键由**身份**唯一决定（`Mate::key`）——转发登记全靠这条投影。
//   · **转发容量**：一枚成员键最多被 `FWD_MAX` 个组关心；满了**显式失败并回滚**，
//     不留"挂着却叫不醒"的半截状态（`permit::fanout`）。
//   · **取用顺序**：死活先于权限——同一个已封印的 token 不因动词换答案
//     （`permit::order`）。
#![cfg(debug_assertions)]

use alloc::vec::Vec;

use env::{Fail, HoleDir, Mark, PieToken, TaskId};

use crate::work::mail::tole::Mate;
use crate::work::mail::{hole, nole, tole};
use crate::work::room::messenger::{FWD_MAX, WakeKey};
use crate::work::room::scheduler::core::prune_dead;
use crate::work::unit::gate::{self, AnyPie, Need, Permission};
use crate::work::unit::space::SpaceBuilder;
use crate::work::unit::team::TeamBuilder;

/// 形态位：一致才放行；`ONLY` 不可撤（自持枚与借入枚同罪）。
pub fn form() {
    let shared = Permission::FETCH | Permission::STORE | Permission::VEST;
    let sole = shared | Permission::ONLY;

    // 四格：一致的两格放行，不一致的两格拒。
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

    // `ONLY` 不可撤：**自持枚（sire = None）也不例外**。
    let name = Mark::of("permit");
    let meta = hole::meta(TaskId::new(0), name);
    for sire in [None, Some(PieToken::mint(1))] {
        let mut pie = AnyPie::Hole(gate::new_pie(meta.clone(), sole, sire));
        crate::expect!(
            gate::narrow(&mut pie, sole).is_ok(),
            "重写同一个权限集应当通过（sire = {:?}）",
            sire
        );
        crate::expect!(
            matches!(gate::narrow(&mut pie, shared), Err(Fail::Denied)),
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
                Err(Fail::Denied)
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

/// 组成员：两种成员都挂得上、同成员幂等、快照按存活过滤、键由身份唯一决定。
///
/// 收尾顺带走一遍组的 `Drop`（撤全部转发登记 + `wipe` 自己的键）——那正是"组没了，
/// 成员那一侧不该再记得它"那条契约的落点。
pub fn members() {
    let group = tole::meta(TaskId::new(0));
    let mark = Mark::of("mate");
    let hole = hole::meta(TaskId::new(0), mark);
    let bell = nole::NoleMeta::new(TaskId::new(0));

    let hole_mate = Mate::Hole(hole.id(), HoleDir::Pull);
    let bell_mate = Mate::Nole(bell.id());

    // 键由身份唯一决定：转发登记、撤登记、退役三处都读它。
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

    // **格子留到成员死**：持有者不做任何清理（格子表不记谁挂的，也就没人替谁收），
    // 成员自己消失才让那一格失效——`cells()` 按存活过滤。
    drop(bell);
    crate::expect!(
        group.cells().is_empty(),
        "铃没了之后，它那一格不该再出现在快照里"
    );
}

/// 转发容量：一枚成员键最多被 `FWD_MAX` 个组关心。
///
/// 三条一起证（判据都在**容量边界**上，不在别处）：
///   ① 前 `FWD_MAX` 个组都挂得上——每个组各占一格转发登记；
///   ② 第 `FWD_MAX + 1` 个**报 `OoM` 且把刚挂的那一格退回**——不留"挂着却叫不醒"的
///      半截状态（那条状态的后果是无限等的人睡到天荒地老）；
///   ③ 那一次被拒**不动既有组的格子**。
///
/// 组用 `Vec` **持着**：`ToleMeta::drop` 会撤掉自己那格转发登记，松了手容量就白测。
/// 「满」是**容量**账（同一枚成员被多少个组关心），不是内存不足——见 `FWD_MAX` 定义处。
pub fn fanout() {
    let mark = Mark::of("member");
    let hole = hole::meta(TaskId::new(0), mark);
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
        matches!(tole::attach(&extra, mate, hole.life()), Err(Fail::OoM)),
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

/// 取用顺序：**死活先于权限**——同一个已封印的 token 不因动词换一个答案。
///
/// 这条此前是每个动词各自的纪律，漏成了两处：`Narrow` 把"覆盖子集"排在死活之前，数据轴
/// 四个动词把"权限"排在死活之前。判据下沉到 `gate::{locate, accede}` 之后，这里直接问
/// 那一处：造一个任务、往它表里放两枚门闩——
///   ① **已封印**且权也不够 ⇒ 必须 `Dead`（两种失败同时在场，看谁先答）；
///   ② 活着但权不够 ⇒ `Denied`；权够 ⇒ 取到；
///   ③ `locate` 不过闸：已封印的那一枚也定位得到（`Release` / `Reserve` 靠它活着）。
///
/// 收尾照 `shell` 那两步簿记清理——留下的未放行线程会让"所有任务都退场"永远不成立。
pub fn order() {
    const USER_BASE: usize = 0x4000_0000;
    let space = SpaceBuilder::user().build().expect("order: build space");
    space.with_flush(|inner| inner.dynamic(USER_BASE));
    let team = TeamBuilder::new(space).spawn().expect("order: spawn team");
    let task = team.task().hold().expect("order: hold task");

    let dead_meta = hole::meta(TaskId::new(0), Mark::of("sealed"));
    hole::seal(&dead_meta);
    let dead = gate::new_pie(dead_meta, Permission::FETCH, None);
    let dead_token = dead.token;
    let live = gate::new_pie(
        hole::meta(TaskId::new(0), Mark::of("live")),
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
            gate::accede(&task, dead_token, Need::Store),
            Err(Fail::Dead)
        ),
        "已封印 + 权不够：必须答 Dead（死活先于权限）"
    );
    crate::expect!(
        matches!(
            gate::accede(&task, live_token, Need::Store),
            Err(Fail::Denied)
        ),
        "活着但权不够：必须答 Denied"
    );
    crate::expect!(
        matches!(gate::accede(&task, live_token, Need::Fetch), Ok(_)),
        "活着且权够：必须取到"
    );
    crate::expect!(
        matches!(gate::locate(&task, dead_token), Ok(_)),
        "locate 不过闸：已封印的那一枚也定位得到"
    );
    crate::expect!(
        matches!(
            gate::locate(&task, PieToken::mint(live_token.get() + 4_096)),
            Err(Fail::Denied)
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
