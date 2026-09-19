// 健康检查 · permit —— 权柄代数的**形态位**（`ONLY`）与组的**成员投影**。
//
// 两件事在这里被证（都只经公开接口，不碰内部字段）：
//
//   · **形态位不是选择，是一致性**（`gate::form_ok`）+ **`ONLY` 不可撤**
//     （`gate::narrow`，**自持枚也不例外**）——这两条是"独占资源复制不出去"的两条腿；
//     缺任一条，`ONLY` 就退回成一句空声明（旧 `CAGE` 的读法就是这么退化的）。
//   · **组的成员投影**：两种成员（孔 / 铃）都挂得上、同成员幂等、快照按存活过滤，
//     且各自的等待键由**身份**唯一决定（`Mate::key`）——转发登记全靠这条投影。
#![cfg(any(debug_assertions, feature = "framework"))]

use env::{HoleDir, Name};

use crate::work::mail::tole::Mate;
use crate::work::mail::{hole, nole, tole};
use crate::work::room::messenger::WakeKey;
use crate::work::unit::gate::{self, AnyPie, GateError, Permission};

/// 形态位：一致才放行；`ONLY` 不可撤（自持枚与借入枚同罪）。
pub(super) fn form() {
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
    let name = Name::new("permit").expect("mark 装得下");
    let meta = hole::meta(0, name);
    for sire in [None, Some(1)] {
        let mut pie = AnyPie::Hole(gate::new_pie(meta.clone(), sole, sire));
        crate::expect!(
            gate::narrow(&mut pie, sole).is_ok(),
            "重写同一个权限集应当通过（sire = {:?}）",
            sire
        );
        crate::expect!(
            matches!(gate::narrow(&mut pie, shared), Err(GateError::Denied)),
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
                Err(GateError::Denied)
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
pub(super) fn members() {
    let group = tole::meta(0);
    let mark = Name::new("mate").expect("mark 装得下");
    let hole = hole::meta(0, mark);
    let bell = nole::NoleMeta::new(0);

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

    tole::hang(&group, hole_mate, hole.life()).expect("挂孔");
    crate::expect!(group.cells().len() == 1, "挂一格之后快照应当有一格");
    tole::hang(&group, hole_mate, hole.life()).expect("重复挂孔");
    crate::expect!(group.cells().len() == 1, "同成员幂等：重复挂不叠加");

    tole::hang(&group, bell_mate, bell.life()).expect("挂铃");
    crate::expect!(
        group.cells().len() == 2,
        "两种成员各占一格（孔的另一个方向可再占一格）"
    );

    tole::unhang(&group, hole_mate).expect("摘孔");
    crate::expect!(group.cells().len() == 1, "摘掉一格后只剩铃那一格");
    tole::unhang(&group, hole_mate).expect("重复摘孔");
    crate::expect!(
        group.cells().len() == 1,
        "没挂过即无事：重复摘不报错也不多加"
    );
}
