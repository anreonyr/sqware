// 能力/资源门闩（gate）— 任务持有什么资源、各什么权限、能否转授/收窄/收回。
//
// 与 `mail` 的分工：gate 只持**能力模型**（门闩 Pie、权限、授权原语），不碰资源
// 实体（HoleMeta/PoleMeta 在 mail）与 IPC 数据面（push/pull/map/unmap 在 mail）。
// `Pie<M>` 泛型直指 `mail` 的 Meta 类型，并持其**唯一强引用**（`Arc<M>`，资源寿命
// = 能力寿命）。
//
// **依赖是单向的**（gate → mail）：唯一的反向边是错误码，而它已在"失败词汇"那一刀搬到
// `env::Fail`——两层从**共同的外部**引它，模块层不再有环。（旧注写"单向依赖 mail，成
// DAG"，当时并不成立：`mail` 的四个模块各自引 `gate::GateError`。）
//
// **派生关系只存一条边**（`sire`）：向上的授与人、向下的子门闩都是查询（`snap`），
// 吃同一张全世界任务快照——快照由适配层拍、boot 注入，gate 不依赖 scheduler。
// `Pie.heir` 不是第二条边：它是"我交出的那一枚"的**本地锚（缓存）**，真相仍在
// `sire` 边上——锚存在的唯一理由是数据面判权不能吃快照（`snap()` 要分配）。
//
// 授权语义在此：`Pie::{allows, covers}` 判定「哪个操作需哪些权利位」
// （`Need::{Fetch,Store,Grant}`）+ 覆盖子集（narrow/accord 共用）。`Grant` 只看
// `VEST`（传递族唯一的目标位）；`ONLY` 是**形态位**、不授予任何事——它是资源事实
// （这枚资源允不许多个使用者），`accord` 只**校验** `subset` 与源枚一致，一致时写锚
// （那次是移交）。envcall 适配层只「取本核 → 转发」，不在壳内重写规则。
//
//   pie.rs     — 门闩（Pie<M>, AnyPie）+ 权限（Permission）+ 操作授权
//                 （Need/allows/covers）+ 错误（Fail）
//   snap.rs    — 全世界任务快照 + 沿 sire 的查询（heirs/vestor/find）
//   accord.rs  — 转授 / 交出给其他 Task（写派生边 + 写锚）+ `clear_heir`
//   narrow.rs  — 就地单调收窄本 pie 权限（`ONLY` 不可撤）
//   cull.rs    — 级联撤销（cull）+ 退出钩子（doom）
//   revoke.rs  — 撤销授与他人的副本（含全部后代）
//   release.rs — 自释自己持有的一份（含全部后代）
//
// 用户态：Task 持 `Vec<AnyPie>`（`unit::task::pies`）；envcall 以 token 寻址。
// `Permission` 单一真相在 `env`（本层 re-export）；错误码契约见**属主域的词表**（`env::fid`）。

mod accord;
mod cull;
mod fail;
mod narrow;
mod pie;
mod release;
mod revoke;

pub(crate) use fail::GateFail;
mod snap;

pub(crate) use pie::{AnyPie, Need, Permission, Pie, accede, locate, new_pie};
// `form_ok` 只有 `accord`（走 `super::pie::` 直呼）与 `health::permit` 两条读者，而后者
// 在 `debug_assertions` 之外不编 ⇒ 无条件重导出会在 release 档报
// `unused import`。门控它，而不是让 release 背一条假警告。
#[cfg(debug_assertions)]
pub(crate) use pie::form_ok;

pub(crate) use accord::{accord, clear_heir};
pub(crate) use cull::{cull, doom};
pub(crate) use narrow::narrow;
pub(crate) use release::release;
pub(crate) use revoke::revoke;
pub(crate) use snap::{install, snap, vestor};
