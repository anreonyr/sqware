// 能力/资源门闩（gate）— 任务持有什么资源、各什么权限、能否转授/收窄/收回。
//
// 与 `mail` 的分工：gate 只持**能力模型**（门闩 Pie、权限、授权原语），不碰资源
// 实体（HoleMeta/PoleMeta 在 mail）与 IPC 数据面（push/pull/map/unmap 在 mail）。
// `Pie<M>` 泛型直指 `mail` 的 Meta 类型；`new_pie` 需 `mail::ResourceId` —— gate
// 单向依赖 mail，成 DAG（无环）。
//
// **派生关系只存一条边**：每枚门闩记 `sire`（父门闩的 token）。向上的授与人、
// 向下的子门闩都是查询（`snap`），吃同一张全世界任务快照——快照由适配层拍、
// boot 注入，gate 不依赖 scheduler。
//
// 授权语义在此：`Pie::{allows, covers}` 判定「哪个操作需哪些权利位」
// （`Need::{Read,Write,Grant}`）+ 覆盖子集（narrow/accord 共用）；BACK 守门
// （带 BACK 只能授回 sire 的持有者）在 `snap::vestable`；envcall 适配层只
// 「取本核 → 转发」，不在壳内重写规则。
//
//   pie.rs     — 门闩（Pie<M>, AnyPie）+ 权限（Permission）+ 操作授权
//                 （Need/allows/covers）+ 错误（GateError）
//   snap.rs    — 全世界任务快照 + 沿 sire 的查询（heirs/vestor/vestable/find）
//   accord.rs  — 转授子集给其他 Task（写派生边）
//   narrow.rs  — 就地单调收窄本 pie 权限
//   cull.rs    — 级联撤销（cull）+ 退出钩子（doom）
//   revoke.rs  — 撤销授与他人的副本（含全部后代）
//   release.rs — 自释自己持有的一份（含全部后代）
//
// 用户态：Task 持 `Vec<AnyPie>`（`unit::task::pies`）；envcall 以 token 寻址。
// `Permission` 单一真相在 `env`（本层 re-export）；错误码契约见 [`GateError::code`]。

mod accord;
mod cull;
mod narrow;
mod pie;
mod release;
mod revoke;
mod snap;

pub(crate) use pie::{AnyPie, GateError, Need, Permission, Pie, new_pie};

pub(crate) use accord::accord;
pub(crate) use cull::{cull, doom};
pub(crate) use narrow::narrow;
pub(crate) use release::release;
pub(crate) use revoke::revoke;
pub(crate) use snap::{install, snap, vestable, vestor};
