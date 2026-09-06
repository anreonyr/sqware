// Domain — 运行期装载 ELF 成独立保护域（spawn_team 内核原语）。
//
// 与 boot 的 `load_user` 同一条拼装流水线（parse → SpaceBuilder::user → loader::load →
// TeamBuilder::spawn），差别只在：本模块是**运行期调用**（用户态 `UnitCall::SpawnTeam`
// 触发），且建立**血缘**（`sire` → 生我者的 task；`sire_upgrade.team.heir` → 我生的子域）。
//
// 核心/适配分离：本模块是「拼装 + 血缘」的适配层；loader 只做「ELF → 装好段的 Space」
// 纯装载，不感知血缘/Team/调度。
//
// `spawnee_elf()` 也在此实现——`include_bytes!(env!("USER_*"))` 需 kernel build.rs 的
// `USER_*` 环境变量（ubi 无该 env，故 Spawnee 枚举在 ubi、取字节在本模块）。

use alloc::sync::{Arc, Weak};

use hashbrown::HashMap;
use ubi::Spawnee;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::unit::elftable::ElfTable;
use crate::work::unit::space::SpaceBuilder;
use crate::work::unit::task::Task;
use crate::work::unit::{loader, parser, team};

/// 运行期域强持有表（id → Arc<Team>）。
///
/// spawn_team 建的域**无自有 task**（spawn_team 不产 task、只有一个 Weak 的
/// `team_table`），若不强持有，`spawn_task` 查到前域已随最后 Arc drop 而死。
/// 故本表**强持有**这些子域（类比 `memo` 对资源实体的强持有）；级联回收 /
/// 显式 reclaim 时 `remove`。
fn domains() -> &'static SpinLock<HashMap<usize, Arc<team::Team>>> {
    static T: OnceLock<SpinLock<HashMap<usize, Arc<team::Team>>>> = OnceLock::new();
    T.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// Spawnee → 内嵌 ELF 字节（编译期 match；文件系统出现后此函数弃用）。
/// 独立自由函数（非 `impl Spawnee`——`elf()` 不能是 ubi 类型的 inherent impl）。
fn spawnee_elf(which: Spawnee) -> &'static [u8] {
    match which {
        Spawnee::Lisp => include_bytes!(env!("USER_LISP")),
        Spawnee::Shell => include_bytes!(env!("USER_SHELL")),
        Spawnee::Back => include_bytes!(env!("USER_BACK")),
        Spawnee::Narrow => include_bytes!(env!("USER_NARROW")),
    }
}

/// spawn_team 结果错误（拼装失败）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainError {
    /// parser / SpaceBuilder / loader 任一步失败。
    Load,
}

/// 装载镜像成独立域（新 Space+Team，不产 task）。
///
/// 血缘：`sire` 是生我者的 task（`Weak`，溯源）；`sire` 所在 team 的 `heir` 记本子域
/// （级联回收入口）。成功返回域句柄（`Arc<Team>`，其 `id` 已是可查的 TeamId）。
///
/// # Errors
/// - `Load` — 拼装任一步失败（不落，无脏域）。
pub(crate) fn spawn_team(which: Spawnee, sire: Weak<Task>) -> Result<Arc<team::Team>, DomainError> {
    let elf = spawnee_elf(which);
    let parsed = parser::parse(elf).map_err(|_| DomainError::Load)?;
    let space = SpaceBuilder::user().build().map_err(|_| DomainError::Load)?;
    let loaded = loader::load(space, elf, &parsed).map_err(|_| DomainError::Load)?;

    let elftable = parser::tables(elf)
        .ok()
        .and_then(|(s, ss)| ElfTable::from_sections(s, ss))
        .map(Arc::new);

    let team = team::TeamBuilder::new(loaded.space).elftable(elftable).spawn();

    // 域记住默认执行入口（装载 ELF 的 e_entry；spawn_task 的 entry=0 时用它）。
    team.set_default_entry(loaded.entry.as_usize());

    // 血缘：sub team 记 sire（生我者）；sire 所在 team 的 heir 记本子域。
    team.set_sire(sire.clone());
    if let Some(sire_arc) = sire.upgrade() {
        sire_arc.ident.team.heir.lock().push(Arc::downgrade(&team));
    }

    // 强持有：子域无自有 task，仅靠本表撑住，否则 `spawn_task` 查到前已死。
    domains().lock().insert(team.id.get(), team.clone());

    Ok(team)
}
