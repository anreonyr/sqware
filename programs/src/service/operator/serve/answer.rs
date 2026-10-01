//! 客人的一句问 → 树那七条原语，编出一句答。
//! **三道闸，次序即契约**：
//! 1. **这一位**（操作面那一维）：会话拿的是哪一位，就只许那一条原语——七位各是一条独立的权柄
//!    边界（`find` 会**交出能力**、`trim` / `part` 会**毁掉别人那一格**）。**它绝不替代下一道**：
//!    拿到 `find` 那一位只表示"许调 `find` 这一类"，不表示"许 `find` 任意一格"。
//! 2. **门外那一问**（super::door::may）：两条会**交出权柄 / 毁掉别人那一格**的原语先过门禁。
//! 3. **那一格自己的两轴**：**用**那一轴由 Operator::permit 答（许可跟着那一枚砖走，
//!    `find` 判它）；**改**那一轴由 Operator::claimable 答（`land` / `part` / `trim` 判它）。
//!    两轴**都住在砖上**，而**分开住**——混成一格就会得出"能改的人自然能用"。它俩都**不问外面**：
//!    许可是一个值，归属是树自己一次查表（唯一要问外边的那一句是"主人还在不在场"，而那是**读**
//!    内核盖的那一格，不推不收）。

use protocol::debug;
use protocol::service::operator as ocall;
use protocol::service::operator::{Grant, Permit};
use runtime::core::res::port::{self, Access, Policy};
use runtime::env::mail;

use crate::service::operator::core::{Key, Operator};

use super::door::may;
use super::watch::{Watchers, event_at};

/// **一次真改动之后**：把那条路走出来、编成一条事件、发给订得起的人。
///
/// 路从**号**现走（`road_to`），剪掉那一档例外——它的路在剪之前就记在 `Change` 里了。
/// 走不出路（号不在树上）就**不发**：一条路是假的事件比没有更坏。
fn changed(tree: &Operator, watchers: &mut Watchers, change: &crate::service::operator::core::Change) {
    let Some(ev) = event_at(tree, change.kind, change.id, change.owner, change.road.clone()) else {
        debug!("operator: watch event unsigned id={}", change.id.get());
        return;
    };
    let _ = watchers.publish(&ev);
}

/// 把一句问交给树，编出一句答（**答话有四种形状**，见 ocall 的帧那一节）
/// **形状由 ocall::Wire 说**（收帧那一侧已经按动作解好了），**答由 ocall::Union 说**
/// 解不出来就是一句读不懂的帧（不猜、不崩）；`land` 那一码**必须带入口号**（没带同样解不出来）
pub(super) fn answer(
    tree: &mut Operator,
    watchers: &mut Watchers,
    ask: Option<ocall::Wire>,
    who: env::TaskId,
    wired: bool,
    grant: Option<Grant>,
) -> ocall::Union {
    // 空帧 / 长度不对 / 表外的动作码：读不懂（答 `BAD`）。
    let Some(ask) = ask else {
        debug!("operator: unreadable frame from={}", who.get());
        return ocall::Union::Status(ocall::BAD);
    };
    if let Some(grant) = grant {
        if grant.at() != Grant::of_wire(&ask) {
            return ocall::Union::Status(ocall::DENIED);
        }
    }
    // **第二道：门外那一问。** 两条会**交出权柄 / 毁掉别人那一格**的原语先过门禁——`find`（把
    // 它的准入是**那一格自己的规矩**（见下面的两支）。四条只读结构的
    // （`part` / `list` / `seek` / `name`）一律不判。
    // **两轴分家**：
    match ask {
        ocall::Wire::Find(id) => {
            let permit = tree.permit(id);
            let ruling = may(tree, wired, who, permit);
            if !ruling.passed() {
                return ocall::Union::Status(ruling.wire());
            }
        }
        ocall::Wire::Trim(id) => {
            if !tree.claimable(Key::Id(id), who) {
                return ocall::Union::Status(ocall::DENIED);
            }
            let ruling = may(tree, wired, who, Permit::Unset);
            if !ruling.passed() {
                return ocall::Union::Status(ruling.wire());
            }
        }
        // **`land` 也要先问身份**（与 `find`/`trim` 同一道门）：它虽然不动别人的格子，
        // 但"往树上挂东西"这件事本身要求来的人是个**已绑身份**——否则没身份的任务就能往命名
        // 空间里塞条目。
        ocall::Wire::Land { .. } => {
            let ruling = may(tree, wired, who, Permit::Unset);
            if !ruling.passed() {
                return ocall::Union::Status(ruling.wire());
            }
        }
        // **`watch` 也要先问身份**（与 `land` 同一道门）：订阅是"此后一直看着树"这件事，
        // 没身份的任务不该得到它。
        ocall::Wire::Watch { .. } => {
            let ruling = may(tree, wired, who, Permit::Unset);
            if !ruling.passed() {
                return ocall::Union::Status(ruling.wire());
            }
        }
        _ => {}
    }
    match ask {
        // **两条答号的**：立/分的人自己得知道立成了几号——答案体不是一格状态。
        ocall::Wire::Land {
            at,
            name,
            entry,
            permit,
            mine,
        } => {
            if !tree.claimable(Key::At(at, name.clone()), who) {
                return ocall::Union::Status(ocall::DENIED);
            }
            // **一问一动**：两轴与那一枚砖**一起落**（`tree.land` 那一手的 Node::Tile）——
            // 故"树改了、两轴没记上"这一类**构造上不存在**，这一支没有第二步可漏。
            return match tree.land(at, name.clone(), entry, permit, mine.then_some(who)) {
                Ok(change) => {
                    changed(tree, watchers, &change);
                    ocall::Union::Entry(change.id)
                }
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
        ocall::Wire::Part { at, name } => {
            if !tree.claimable(Key::At(at, name.clone()), who) {
                return ocall::Union::Status(ocall::DENIED);
            }
            return match tree.part_at(at, name) {
                Ok((id, _fresh, change)) => {
                    // **幂等那一档没有事件**（`change = None` = 树一个字节没变），号照答。
                    if let Some(change) = &change {
                        changed(tree, watchers, change);
                    }
                    ocall::Union::Entry(id)
                }
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
        // 查到就**把树上那一份转授给客人**：Pie 本身不从报文里走，从会话里走；而
        // **它在客人表里的号**从这条答话里走（ocall::Union::Seed）——客人拿它一次 `Reserve`
        // 就认得出，不必扫自己的表。
        // "查不到"与"授不出去"是两件事，故查的结论优先：`said` 先答，其次才轮到 `grant`。
        ocall::Wire::Find(id) => {
            let mut seed = None;
            let mut grant = Ok(());
            let said = tree.find(id, |pie| {
                // **交出那一手就是 port::ship**（`R|W` ＋ 一格 `VEST`）：捡到的那一枚砖
                // 要能替客人再授出，少 `VEST` ⇒ 转授那一步答 `Denied`。
                let grant_pie = mail::HolePie::from_token(pie);
                grant = port::ship(&grant_pie, who, Access::FETCH | Access::STORE, Policy::VEST)
                    .map(|at| seed = Some(at.seed()))
                    .map_err(|_| ocall::Fail::Unknown);
            });
            let fail = said.err().or(grant.err());
            return match (fail, seed) {
                (None, Some(seed)) => ocall::Union::Seed(seed),
                (Some(fail), _) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
                // 授成功却没拿到号：这是内核契约破了（`ship` 的成功值就是那一格），不猜。
                (None, None) => ocall::Union::Status(ocall::BAD),
            };
        }
        // `trim` 的两档都是"一格状态"：**各自就地成答**，不再走下面那条窄路（`said`）。
        ocall::Wire::Trim(id) => match tree.trim(id) {
            Ok(change) => {
                if let Some(change) = &change {
                    changed(tree, watchers, change);
                }
                return ocall::Union::Status(ocall::OK);
            }
            Err(fail) => return ocall::Union::Status(ocall::fail_to_code(Some(fail))),
        },
        // **三条答数据的**：答案体不是一格状态，故各自编各自的帧（成败都在帧里）。
        ocall::Wire::List(at) => {
            return match tree.list(at) {
                Ok(ids) => ocall::Union::List(ocall::Listing::of(ids)),
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
        ocall::Wire::Name(id) => {
            return match tree.name(id) {
                Ok(name) => ocall::Union::Name(name),
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
        ocall::Wire::Road(road) => {
            return match tree.seek(&road) {
                Ok(id) => ocall::Union::Entry(id),
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
        // **订一条子树**：把交来的页与铃认成写端，记下"谁订了哪条路"。**答的就是成没成**
        // （订阅者拿这一句当"此后的事件都算你的"那个点——见 `watch` 面那一节的序）。
        ocall::Wire::Watch { road, page, bell } => {
            return match watchers.join(who, &road, page, bell) {
                Ok(()) => ocall::Union::Status(ocall::OK),
                Err(()) => ocall::Union::Status(ocall::fail_to_code(Some(ocall::Fail::Denied))),
            };
        }
    }
}
