//! operator::answer — **适配**：客人的一句问 → 树那七条原语 ＋ 账，编出一句答。
//!
//! 它是这一族**唯一**读那本账（[`Book`]）的地方：门外那一问只判"许不许"（[`super::door`]），
//! 落格那一支只写（[`super::plate`]）。
//!
//! **三道闸，次序即契约**：
//!
//! 1. **这一位**（操作面那一维）：会话拿的是哪一位，就只许那一条原语——七位各是一条独立的权柄
//!    边界（`find` 会**交出能力**、`trim` 会**毁掉别人那一格**）。**它绝不替代下一道**：拿到
//!    `find` 那一位只表示"许调 `find` 这一类"，不表示"许 `find` 任意一格"。
//! 2. **门外那一问**（[`super::door::may`]）：两条会**交出权柄 / 毁掉别人那一格**的原语先过门禁。
//! 3. **那一格自己的两轴**：**用**那一轴由树答（[`Operator::permit`] —— 许可跟着那一枚砖走，
//!    `find` 判它）；**改**那一轴由 [`Ledger::claimable`] 答（`land` / `trim` 判它）。两轴**分开
//!    住**：许可在树上（它是砖的性质），归属在账上；而两轴都**不在核心**的裁决里（核心是同步
//!    纯函数，发不出那两条问身份的消息）。

use protocol::system::operator as ocall;
use protocol::system::operator::{Grant, Permit};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

use crate::system::operator::core::Operator;
use crate::system::operator::core::ledger::{Book, Key, fresh};

use super::bridge::Coord;
use super::door::may;

/// 把一句问交给树，编出一句答（**答话有四种形状**，见 [`ocall`] 的帧那一节）。
///
/// **形状由 [`ocall::Wire`] 说**（收帧那一侧已经按动作解好了），**答由 [`ocall::Union`] 说**：
/// 解不出来就是一句读不懂的帧（不猜、不崩）；`land` 那一码**必须带入口号**（没带同样解不出来）。
pub(super) fn answer(
    tree: &mut Operator,
    ask: Option<ocall::Wire>,
    who: env::TaskId,
    coord: Coord,
    book: &mut Book,
    grant: Option<Grant>,
) -> ocall::Union {
    // 空帧 / 长度不对 / 表外的动作码：读不懂（答 `BAD`）。
    let Some(ask) = ask else {
        return ocall::Union::Status(ocall::BAD);
    };
    // 路太长：**先按上限挡掉**，别把一条被截断的路当成真的（核心那几条原语也各有这条判据）。
    if let ocall::Wire::Road(_, count) = ask {
        if count > ocall::frame::ROAD_MAX {
            return ocall::Union::Status(ocall::FULL);
        }
    }
    // **第一道：这一位。** 会话拿的是哪一位，就只许那一条原语——七位各是一条独立的权柄边界
    // （`find` 会**交出能力**、`trim` 会**毁掉别人那一格**，故它们不与只读那几条合成一位）。
    // `None`（控制面那条路 / 表外记号）⇒ 不判面 ⇒ 今天那几台客人一字不变。
    if let Some(grant) = grant {
        if grant.at() != Grant::of_wire(&ask) {
            return ocall::Union::Status(ocall::DENIED);
        }
    }
    // **第二道：门外那一问。** 两条会**交出权柄 / 毁掉别人那一格**的原语先过门禁——`find`（把
    // 那一枚授出去）与 `trim`（把别人的名字剪掉）。`land` **不在这里**判：它是"改我自己那一格"，
    // 它的准入是**那一格自己的规矩**（见下面的两支）。四条只读结构的
    // （`part` / `list` / `seek` / `name`）一律不判。
    //
    // **两轴分家**：
    //
    // - **用**那一轴（谁许用这一格）跟着那一枚砖走，由 [`Operator::permit`] 答；`find` 判它；
    // - **改**那一轴（谁许改这一格）住在账上，由 [`Book::claimable`] 答；`land` / `trim` 判它。
    match ask {
        // **`find` 看这一格自己的"用"那一轴**。
        //
        // **读是公开的，写才归属主**：改那一轴（`land` 那一格的 `mine`，账里记成 `Owner`）
        // 管的是**改这一格**，不是**用这一格**。
        ocall::Wire::Find(id) => {
            let permit = tree.permit(id);
            let ruling = may(tree, coord, who, permit);
            if !ruling.passed() {
                return ocall::Union::Status(ruling.wire());
            }
        }
        ocall::Wire::Trim(id) => {
            if !book.claimable(Key::Id(id), who, |id| fresh(tree, id)) {
                return ocall::Union::Status(ocall::DENIED);
            }
            let ruling = may(tree, coord, who, Permit::Unset);
            if !ruling.passed() {
                return ocall::Union::Status(ruling.wire());
            }
        }
        // **`land` 也要先问身份**（与 `find`/`trim` 同一道门）：它虽然不动别人的格子，
        // 但"往树上挂东西"这件事本身要求来的人是个**已绑身份**——否则没身份的任务就能往命名
        // 空间里塞条目。
        ocall::Wire::Land { .. } => {
            let ruling = may(tree, coord, who, Permit::Unset);
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
            // **"改这一格"那一轴**：落之前先看这一格现在归谁——不是我就拒。占了的位置由
            // **活着的主人**说了算；空着的位置谁都能落，落了就登记成他的。
            //
            // **按坐标查**（不是按号）：`land` 那一问发生在动树之前，而 `land` 换绑**不动号**
            // ——故那一刻手里只有坐标。
            if !book.claimable(Key::At(at, name), who, |id| fresh(tree, id)) {
                return ocall::Union::Status(ocall::DENIED);
            }
            // **一问一动**：要位 → 落树 → 记账全在 [`Book::land`] 里，漏不掉中间那一步。
            // 账上记的只有归属（`mine`）；**许可与那一枚砖一起落**（`tree.land` 那一手，
            // `permit` 进的是 `Node::Tile`）——故它与树同生，不会"树改了、许可没记上"。
            return match book.land(at, name, entry, mine, who, || {
                tree.land(at, name, entry, permit)
            }) {
                Ok(id) => ocall::Union::Entry(id),
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
        ocall::Wire::Part { at, name } => {
            return match tree.part(at, name) {
                Ok(id) => {
                    // **那个窄口子**：`part` 碰到一枚 `Tile` 会静默把它顶成一块 `Pane`
                    // ——那一格已经不是"放 Pie 的那一格"了，故账上那一行要销掉。
                    // （漏了也不会答错：`fresh` 那一次对真相兜着；这只是不让账留一条陈的。）
                    book.drop(id);
                    ocall::Union::Entry(id)
                }
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
                grant = port::ship(
                    &grant_pie,
                    who,
                    Access::FETCH | Access::STORE,
                    Policy::VEST,
                )
                .map(|at| seed = Some(at.seed()))
                .map_err(|_| ocall::Fail::Unknown);
            });
            if said == Err(ocall::Fail::Dead) {
                // 核心**已经**把那一格剔了（"惰性剔死"）——顺手销账，别留一条陈的。
                book.drop(id);
            }
            // 三步都成 ⇒ 答 `[OK][那一格]`；任何一步没成 ⇒ 照旧一格状态（不猜）。
            let fail = said.err().or(grant.err());
            return match (fail, seed) {
                (None, Some(seed)) => ocall::Union::Seed(seed),
                (Some(fail), _) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
                // 授成功却没拿到号：这是内核契约破了（`ship` 的成功值就是那一格），不猜。
                (None, None) => ocall::Union::Status(ocall::BAD),
            };
        }
        ocall::Wire::Trim(id) => {
            let said = tree.trim(id);
            if said.is_ok() {
                // 格子从树上没了 ⇒ 那一行也走。
                book.drop(id);
            }
            said
        }
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
        ocall::Wire::Road(road, count) => {
            return match tree.seek(&road[..count.min(ocall::frame::ROAD_MAX)]) {
                Ok(id) => ocall::Union::Entry(id),
                Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
            };
        }
    };
    ocall::Union::Status(ocall::fail_to_code(said.err()))
}
