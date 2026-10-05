use runtime::schedule::{Progress, ResMut};
use super::session::Request;
use crate::system::operator::core::{Location, Tile, Key, Operator};
use protocol::{
    system::operator as ocall,
};
use runtime::core::res::pie::HolePie;
use runtime::core::res::port::{self, Access, Policy};
pub(super) struct Output<T> {
    pub reply: Option<T>,
    pub changes: alloc::vec::Vec<crate::system::operator::core::Change>,
}
pub(super) fn apply(
    mut request: ResMut<Request>,
    mut tree: ResMut<Operator>,
    mut out: ResMut<Output<ocall::Union>>,
) -> Result<Progress, super::Fail> {
    if out.reply.is_some() {
        return Ok(Progress::Done);
    }
    let Some(incoming) = &mut request.0 else {
        return Ok(Progress::Done);
    };
    let Some(ask) = incoming.ask.take() else {
        return Ok(Progress::Done);
    };
    let who = incoming.guest.who();
    let result = (|| {
        match ask {
            // **两条答号的**：立/分的人自己得知道立成了几号——答案体不是一格状态。
            ocall::Wire::Land {
                at,
                name,
                entry,
                permit,
                mine,
            } => {
                if !tree.claimable(Key::At(at, &name), who) {
                    return ocall::Union::Status(ocall::DENIED);
                }
                // **一问一动**：两轴与那一枚砖**一起落**（`tree.land` 那一手的 Node::Tile）——
                // 故"树改了、两轴没记上"这一类**构造上不存在**，这一支没有第二步可漏。
                return match tree.land(
                    Location { at, name },
                    Tile {
                        pie: entry,
                        permit,
                        owner: mine.then_some(who),
                    },
                ) {
                    Ok(change) => {
                        let id = change.id;
                        out.changes.push(change);
                        ocall::Union::Entry(id)
                    }
                    Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
                };
            }
            ocall::Wire::Part { at, name } => {
                if !tree.claimable(Key::At(at, &name), who) {
                    return ocall::Union::Status(ocall::DENIED);
                }
                return match tree.part_at(at, name) {
                    Ok((id, _fresh, change)) => {
                        // **幂等那一档没有事件**（`change = None` = 树一个字节没变），号照答。
                        if let Some(change) = change {
                            out.changes.push(change);
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
                    let grant_pie = HolePie::from_token(pie);
                    grant =
                        port::ship(grant_pie.token(), who, Access::FETCH | Access::STORE, Policy::VEST)
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
                    if let Some(change) = change {
                        out.changes.push(change);
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
            ocall::Wire::Watch { .. } => ocall::Union::Status(ocall::BAD),
        }
    })();
    out.reply = Some(result);
    Ok(Progress::Done)
}
