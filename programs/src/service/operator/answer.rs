//! operator::answer — **适配**：客人的一句问 → 树那七条原语，编出一句答。
//! **三道闸，次序即契约**：
//! 1. **这一位**（操作面那一维）：会话拿的是哪一位，就只许那一条原语——七位各是一条独立的权柄
//!    边界（`find` 会**交出能力**、`trim` / `part` 会**毁掉别人那一格**）。**它绝不替代下一道**：
//!    拿到 `find` 那一位只表示"许调 `find` 这一类"，不表示"许 `find` 任意一格"。
//! 2. **门外那一问**（[`super::door::may`]）：两条会**交出权柄 / 毁掉别人那一格**的原语先过门禁。
//! 3. **那一格自己的两轴**：**用**那一轴由 [`Operator::permit`] 答（许可跟着那一枚砖走，
//!    `find` 判它）；**改**那一轴由 [`Operator::claimable`] 答（`land` / `part` / `trim` 判它）。
//!    两轴**都住在砖上**，而**分开住**——混成一格就会得出"能改的人自然能用"。它俩都**不问外面**：
//!    许可是一个值，归属是树自己一次查表（唯一要问外边的那一句是"主人还在不在场"，而那是**读**
//!    内核盖的那一格，不推不收）。

use protocol::debug;
use protocol::service::operator as ocall;
use protocol::service::operator::{Grant, Permit};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

use crate::service::operator::core::{Key, Operator};

use super::door::may;

/// 把一句问交给树，编出一句答（**答话有四种形状**，见 [`ocall`] 的帧那一节）。
/// **形状由 [`ocall::Wire`] 说**（收帧那一侧已经按动作解好了），**答由 [`ocall::Union`] 说**：
/// 解不出来就是一句读不懂的帧（不猜、不崩）；`land` 那一码**必须带入口号**（没带同样解不出来）。
pub(super) fn answer(
    tree: &mut Operator,
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
    // 那一枚授出去）与 `trim`（把别人的名字剪掉）。`land` **不在这里**判：它是"改我自己那一格"，
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
        _ => {}
    }
    let said = match ask {
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
            // **一问一动**：两轴与那一枚砖**一起落**（`tree.land` 那一手的 `Node::Tile`）——
            // 故"树改了、两轴没记上"这一类**构造上不存在**，这一支没有第二步可漏。
            // `mine` 线上那一格是裸布尔，在这里折成砖上那 `owner` 一格（`None` = 不留主人）。
            return match tree.land(at, name, entry, permit, mine.then_some(who)) {
                Ok(id) => ocall::Union::Entry(id),
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
        ocall::Wire::Part { at, name } => {
            if !tree.claimable(Key::At(at, name.clone()), who) {
                return ocall::Union::Status(ocall::DENIED);
            }
            return match tree.part(at, name) {
                Ok(id) => ocall::Union::Entry(id),
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
        // 查到就**把树上那一份转授给客人**：Pie 本身不从报文里走，从会话里走；而
        // **它在客人表里的号**从这条答话里走（[`ocall::Union::Seed`]）——客人拿它一次 `Reserve`
        // 就认得出，不必扫自己的表。
        // "查不到"与"授不出去"是两件事，故查的结论优先：`said` 先答，其次才轮到 `grant`。
        ocall::Wire::Find(id) => {
            let mut seed = None;
            let mut grant = Ok(());
            let said = tree.find(id, |pie| {
                // **交出那一手就是 `port::ship`**（`R|W` ＋ 一格 `VEST`）：捡到的那一枚砖
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
        ocall::Wire::Trim(id) => tree.trim(id),
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
        // **译号那一档**：名字只能走到这里——拿到号之后，其余原语一律按号走。
        ocall::Wire::Road(road) => {
            return match tree.seek(&road) {
                Ok(id) => ocall::Union::Entry(id),
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
    };
    ocall::Union::Status(ocall::fail_to_code(said.err()))
}
