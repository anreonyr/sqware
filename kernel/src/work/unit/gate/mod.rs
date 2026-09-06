// 能力/资源门闩（gate）— 任务持有什么资源、各什么权限、能否转授/收窄/收回。
//
// 与 `mail` 的分工：gate 只持**能力模型**（门闩 Pie、权限、授权原语），不碰资源
// 实体（HoleMeta/PoleMeta 在 mail）与 IPC 数据面（push/pull/map/unmap 在 mail）。
// `Pie<M>` 泛型直指 `mail` 的 Meta 类型；`new_pie` 需 `mail::ResourceId` —— gate
// 单向依赖 mail，成 DAG（无环）。
//
// 授权语义在此：`Pie::{allows, allows_subset}` 判定「哪个操作需哪些权利位」
// （`Need::{Read,Write,Grant}`）+ 子集合法（narrow/accord 共用）；envcall 适配层
// 只「取本核 → 转发」，不在壳内重写规则。
//
//   pie.rs    — 门闩（Pie<M>, AnyPie）+ 权限（Permission）+ 操作授权（Need/allows）
//                + 错误（GateError）
//   accord.rs — 转授子集给其他 Task
//   narrow.rs — 就地单调收窄本 pie 权限
//   revoke.rs — 收回授与他人的副本
//
// 用户态：Task 持 `Vec<AnyPie>`（`unit::task::pies`）；envcall 以 token 寻址。
// `Permission` 单一真相在 `ubi`（本层 re-export）；错误码契约见 [`GateError::code`]。

mod accord;
mod narrow;
mod pie;
mod revoke;

pub(crate) use pie::{new_pie, AnyPie, GateError, Need, Permission, Pie};

pub(crate) use accord::accord;
pub(crate) use narrow::narrow;
pub(crate) use revoke::revoke;
