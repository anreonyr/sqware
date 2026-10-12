use super::events::Watchers;
use super::exchange::Request;
use crate::system::operator::tree::Operator;
use ::resource::port::{self, Policy};
use ::schedule::{Progress, ResMut};
use system_api::operator as ocall;
pub(super) struct Output {
    pub reply: Option<ocall::Union>,
    pub changes: alloc::vec::Vec<crate::system::operator::tree::Change>,
    pub watchers: Watchers,
}
pub(super) fn apply(
    mut request: ResMut<Request>,
    mut tree: ResMut<Operator>,
    mut out: ResMut<Output>,
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
            // 查到就**把树上那一份转授给客人**：Pie 本身不从报文里走，从会话里走；而
            // **它在客人表里的号**从这条答话里走（ocall::Union::Seed）——客人拿它一次 `Reserve`
            // 就认得出，不必扫自己的表。
            // "查不到"与"授不出去"是两件事，故查的结论优先：`said` 先答，其次才轮到 `grant`。
            ocall::Wire::Find(id) => {
                let mut seed = None;
                let mut grant = Ok(());
                let said = tree.find(id, |pie, view| {
                    // Each binding exposes only its declared consumer view.
                    grant = port::ship(pie, who, view, Policy::NONE)
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
            ocall::Wire::Resolve { mark, name } => {
                return match tree.resolve_namespace_parts(mark, &name) {
                    Ok(entry) => ocall::Union::Reference(entry),
                    Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
                };
            }
            ocall::Wire::Relative { base, road } => {
                return match tree.seek_from(base, &road) {
                    Ok(entry) => ocall::Union::Reference(entry),
                    Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
                };
            }
            // **订一条子树**：把交来的页与铃认成写端，记下"谁订了哪条路"。**答的就是成没成**
            // （订阅者拿这一句当"此后的事件都算你的"那个点——见 `watch` 面那一节的序）。
            ocall::Wire::Watch { .. } | ocall::Wire::WatchFrom { .. } => {
                ocall::Union::Status(ocall::BAD)
            }
        }
    })();
    out.reply = Some(result);
    Ok(Progress::Done)
}
