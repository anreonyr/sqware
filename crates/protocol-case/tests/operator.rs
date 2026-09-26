//! operator 核心的门（**宿主台**）—— 树与七条原语的规矩，在宿主上真跑一遍。
//!
//! # 这批判据为什么住在这里
//!
//! `protocol` 是 `[lib] test = false`（目标 riscv64gc-unknown-none-elf 上编不出 libtest），
//! 故 `cargo check --workspace --all-targets` 与 `build --release` **都不编**它的
//! `#[cfg(test)]`；而**链接 `protocol` 去测**也走不通——它依赖 `runtime`，`runtime` 里那两处
//! riscv 内联汇编在宿主编译器上编不出来（见本 crate 的 `Cargo.toml`，那里记着实测读数）。
//!
//! 故这一台与 `crates/alloc-probe` 同路：宿主 crate、**真依赖**「约」`contract`——
//! `crates/contract/src/system/operator/core/mod.rs`（树）与 `crates/contract/src/id.rs`（号的词汇）。
//!
//! **照实记（`#[path]` 退场）**：这一台原先用 `#[path]` 把那两份逐字编进靶。
//! 用 `#[path]` 而不是 `include!` 的另一条**照实记**：`include!` 那一版第一跑就红，
//! `error[E0753]: expected outer doc comment` 报了六次——被编进来的文件以 `//!` 开头
//! （模块内文档），而宏展开出来的内部属性落不到"模块体的开头"那个位置上；`#[path]` 把它当
//! 一个真正的模块文件读，`//!` 就正正经经是那个模块的文档。分家之后（那两份住 `contract`，
//! 只依赖 `env` 与 `plan`）`#[path]` 也不再需要：**那一份源码本来就是模块**，真依赖即可。
//!
//! # 这批判据钉的是什么
//!
//! **号是唯一的直接坐标**：`land` / `part` 答**那一格自己的号**（换绑不动号），`find` / `trim` /
//! `name` 收号，`list` / `land` / `part` 收容器坐标 [`Where`]（根或号）；名字只到 `seek` 那一格
//! ——故 `names` / `look` 两个助手都**先 `seek` 再按号走**，那就是线上那一趟的形状。
//!
//! 照实记：这批用例是从 `core.rs` 的 `#[cfg(test)]` 模块**搬**过来的；搬之前它们一行都没被跑过。
//! 从今以后每一次改核心，这批评据都会先响——**没有门的档 = 没有编译过的档**。

extern crate alloc;

/// 树的正文——**真依赖** `contract::system::operator::core`（逐字同一份源码）。
/// **模块名就叫 `operator`**：靶里那些 `use crate::operator::…` 一字不改。
///
/// **照实记（这一台原先还要一片 `id`）**：树那一份写着 `use crate::id::Id`，`#[path]` 那一版
/// 里那个 `crate` 指的是**靶**的根 ⇒ 靶得把 `contract/src/id.rs` 也编一份进来。真依赖之后
/// 那一行落在 `contract` 的根上，**靶不再需要它**。
use contract::system::operator::core as operator;

use core::sync::atomic::{AtomicUsize, Ordering};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Mutex;

use env::{Name, PieToken, TaskId};

use crate::operator::{EntryId, Fail, Operator, Stamps, Where};

// ── 一台"分配会失败"的台子（只给下面那一条用例）──────────────────
//
// 树那一侧有一格判据是"**备不下就如实报**"（`try_reserve → Fail::Full`）。它只有把分配**真的
// 打掉**才量得到——故这里给测试靶换一个**可关掉**的全局分配器：
//
// - 默认**原样转发**系统分配器（别的用例一字不受影响）；
// - 旗帜是**线程局部**的，不是进程级的：libtest 每个用例各一枚线程，而"打掉分配"若做成全局
//   旗帜，那一条用例亮旗的时候别的用例正好在分配 ⇒ 随机 panic（而随机红的门比没有门更坏）；
// - 用 `const` 初值：`thread_local!` 的惰性初始化**本身要分配**——在分配器里自指。`Cell<bool>`
//   没有析构，故这条线程局部不注册析构器，也不分配。
struct Flaky;

thread_local! {
    static NO_ROOM: Cell<bool> = const { Cell::new(false) };
}

unsafe impl GlobalAlloc for Flaky {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if NO_ROOM.with(Cell::get) {
            // 内核里那一条是 `handle_alloc_error`（abort）——宿主上等价的就是**返空**，
            // 让 `Vec::try_reserve` 如实答 `Err`。
            return core::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        if NO_ROOM.with(Cell::get) {
            return core::ptr::null_mut();
        }
        unsafe { System.realloc(ptr, layout, new) }
    }
}

#[global_allocator]
static ALLOC: Flaky = Flaky;

/// 假表是**进程级**的（`VestedBy` / `Unship` 是函数指针，捕不了环境），故测试彼此串行。
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// 假表：第 i 位非 0 ⇒ 令牌 i 还答得出（"授与人是谁"本正文用不到）。
///
/// 比 `PANE_CAP` **多一位**：撑满一棵树要 `PANE_CAP` 枚都答得出的令牌，
/// 第 `PANE_CAP` 位那个号也在表里——"树满了"必须是 `Full`，不能先被活性挡住。
static TABLE: [AtomicUsize; Operator::PANE_CAP + 1] =
    [const { AtomicUsize::new(0) }; Operator::PANE_CAP + 1];
static FREED: AtomicUsize = AtomicUsize::new(0);

/// 造一枚号给核心用：**唯一的门是"收号"**（`PieToken::from_bytes`）。
fn tok(n: usize) -> PieToken {
    PieToken::from_bytes(&(n as u64).to_le_bytes()).expect("8 字节")
}

fn fake_vested_by(entry: PieToken) -> Option<TaskId> {
    match entry.get() {
        at if at < TABLE.len() => match TABLE[at].load(Ordering::Relaxed) {
            0 => None,
            who => Some(TaskId::new(who)),
        },
        _ => None,
    }
}

/// 假表第二枚戳子：**开者**（与"授与人"分开编号，好让判据里两枚戳子分得开）。
///
/// 照实记：这一格读的是**同一张活表**——真机上 `owner` 与 `vestor` 是两格，而"这扇门封印了"
/// 那件事两枚戳子同时答不出（`Reserve` 的存活闸是共享的）。故 `gone(n)` 在 [`Operator::opens`]
/// 那一格里就是 [`Fail::Dead`]，而那正是要量的那一格。
fn fake_opened_by(entry: PieToken) -> Option<TaskId> {
    fake_vested_by(entry).map(|who| TaskId::new(who.get() + OPENED_BIAS))
}

/// 开者那一格与授与人那一格的编号偏置（两枚戳子同型 ⇒ 这一格是"分得开"的判据）。
const OPENED_BIAS: usize = 1000;

/// 记下"这一枚被放下了"。假表有 `PANE_CAP + 1` 位，越界的令牌不记。
fn fake_unship(entry: PieToken) -> Result<(), ()> {
    if entry.get() < TABLE.len() {
        FREED.fetch_or(1usize << entry.get(), Ordering::Relaxed);
    }
    Ok(())
}

fn tree() -> Operator {
    for slot in &TABLE {
        slot.store(0, Ordering::Relaxed);
    }
    FREED.store(0, Ordering::Relaxed);
    Operator::new(
        Stamps {
            vested_by: fake_vested_by,
            opened_by: fake_opened_by,
        },
        fake_unship,
    )
}

/// 令牌 `entry` 还答得出。
fn live(entry: usize) {
    if let Some(slot) = TABLE.get(entry) {
        slot.store(1, Ordering::Relaxed);
    }
}

/// 那枚不在了（后面的人没了）。
fn gone(entry: usize) {
    if let Some(slot) = TABLE.get(entry) {
        slot.store(0, Ordering::Relaxed);
    }
}

fn unshipped(entry: usize) -> bool {
    FREED.load(Ordering::Relaxed) & (1usize << entry) != 0
}

fn name(text: &str) -> Name {
    Name::new(text).unwrap()
}

fn path(texts: &[&str]) -> Vec<Name> {
    texts.iter().map(|t| name(t)).collect()
}

/// 一条路 → 容器坐标：**先 `seek` 译成号**（空路 = 根），此后按号。行上就是这么走的。
fn at(op: &Operator, road: &[Name]) -> Result<Where, Fail> {
    match road.is_empty() {
        true => Ok(Where::Root),
        false => Ok(Where::At(op.seek(road)?)),
    }
}

/// 列那一块 `Pane`，把号换回名字，收成 `Vec`（免得到处写 `.collect::<Vec<_>>()`）。
///
/// **走的就是线上那一趟**：先 `seek` 收坐标、`list` 收号，再逐枚 `name`——故这批判据顺带把
/// 「路 → 号」与「号 ↔ 名」这两对对得起来也记下来了。
fn names(op: &Operator, road: &[Name]) -> Result<Vec<Name>, Fail> {
    let ids: Vec<EntryId> = op.list(at(op, road)?)?.collect();
    ids.into_iter().map(|id| op.name(id)).collect()
}

/// 寻一条路：**先译成号，再按号寻**，把交出来的那一枚记下来。
fn look(op: &mut Operator, road: &[Name]) -> Result<Option<PieToken>, Fail> {
    let id = op.seek(road)?;
    let mut got = None;
    op.find(id, |pie| got = Some(pie)).map(|()| got)
}

#[test]
fn a_tile_lands_and_find_hands_it_back() {
    let _serial = serial();
    let mut t = tree();
    live(1);
    assert_eq!(
        t.land(Where::Root, name("uart0"), tok(1)),
        Ok(EntryId::new(0))
    );
    assert_eq!(look(&mut t, &path(&["uart0"])), Ok(Some(tok(1))));
    assert_eq!(names(&t, &[]), Ok(std::vec![name("uart0")]));
}

#[test]
fn a_pane_opens_a_second_level() {
    let _serial = serial();
    let mut t = tree();
    assert_eq!(t.part(Where::Root, name("dev")), Ok(EntryId::new(0)));
    live(1);
    assert_eq!(
        t.land(Where::At(EntryId::new(0)), name("uart0"), tok(1)),
        Ok(EntryId::new(1))
    );
    assert_eq!(look(&mut t, &path(&["dev", "uart0"])), Ok(Some(tok(1))));
    assert_eq!(names(&t, &path(&["dev"])), Ok(std::vec![name("uart0")]));
    assert_eq!(names(&t, &[]), Ok(std::vec![name("dev")]));
}

#[test]
fn a_missing_id_is_unknown() {
    let _serial = serial();
    let mut t = tree();
    live(1);
    // 一枚没铸过的号：**作为条目**（`land` 的容器 / `trim`）与**作为一条路**都答同一格。
    assert_eq!(
        t.land(Where::At(EntryId::new(7)), name("uart0"), tok(1)),
        Err(Fail::Unknown)
    );
    assert_eq!(
        t.part(Where::At(EntryId::new(7)), name("sub")),
        Err(Fail::Unknown)
    );
    assert_eq!(t.trim(EntryId::new(7)), Err(Fail::Unknown));
    assert_eq!(t.name(EntryId::new(7)), Err(Fail::Unknown));
    assert_eq!(look(&mut t, &path(&["dev", "uart0"])), Err(Fail::Unknown));
    assert_eq!(names(&t, &path(&["dev", "uart0"])), Err(Fail::Unknown));
}

#[test]
fn walking_through_a_tile_is_not_a_pane() {
    let _serial = serial();
    let mut t = tree();
    live(1);
    assert_eq!(
        t.land(Where::Root, name("log"), tok(1)),
        Ok(EntryId::new(0))
    );
    live(2);
    // 拿一枚 `Tile` 当容器 ⇒ 走不进去（落 / 分 / 列 三条都走这一格）。
    assert_eq!(
        t.land(Where::At(EntryId::new(0)), name("x"), tok(2)),
        Err(Fail::NotAPane)
    );
    assert_eq!(
        t.part(Where::At(EntryId::new(0)), name("x")),
        Err(Fail::NotAPane)
    );
    assert_eq!(
        t.list(Where::At(EntryId::new(0))).map(|ids| ids.count()),
        Err(Fail::NotAPane)
    );
    // 路也走不过去（中途那一段是砖），到头是砖则是"列不动"。
    assert_eq!(names(&t, &path(&["log", "x"])), Err(Fail::NotAPane));
    assert_eq!(names(&t, &path(&["log"])), Err(Fail::NotAPane));
}

#[test]
fn finding_a_pane_is_not_a_tile() {
    let _serial = serial();
    let mut t = tree();
    assert_eq!(t.part(Where::Root, name("dev")), Ok(EntryId::new(0)));
    assert_eq!(look(&mut t, &path(&["dev"])), Err(Fail::NotATile));
    // **根递不进 `find`**：它要的是一枚号，而根没有号——这是类型义务，不是运行期那一格。
}

#[test]
fn listing_a_tile_is_not_a_pane() {
    let _serial = serial();
    let mut t = tree();
    live(1);
    assert_eq!(
        t.land(Where::Root, name("log"), tok(1)),
        Ok(EntryId::new(0))
    );
    assert_eq!(names(&t, &path(&["log"])), Err(Fail::NotAPane));
}

#[test]
fn a_pane_with_things_in_it_is_not_moved() {
    let _serial = serial();
    let mut t = tree();
    assert_eq!(t.part(Where::Root, name("dev")), Ok(EntryId::new(0)));
    live(1);
    assert_eq!(
        t.land(Where::At(EntryId::new(0)), name("uart0"), tok(1)),
        Ok(EntryId::new(1))
    );
    live(2);
    // 非空那块 Pane：**会毁掉内容**的那两条不许动它（落 = 换绑、剪），**分是幂等的**。
    assert_eq!(
        t.land(Where::Root, name("dev"), tok(2)),
        Err(Fail::NonEmpty)
    );
    assert_eq!(
        t.trim(EntryId::new(0)),
        Err(Fail::NonEmpty),
        "非空 Pane 剪不动"
    );
    assert_eq!(
        t.part(Where::Root, name("dev")),
        Ok(EntryId::new(0)),
        "分是幂等的：那儿已经是一块 Pane ⇒ 答它那个号，里面一条不动"
    );
    assert!(!unshipped(1), "非空那块 Pane 一根毫毛都没动");
    assert_eq!(look(&mut t, &path(&["dev", "uart0"])), Ok(Some(tok(1))));
}

#[test]
fn a_pane_is_already_what_a_part_asks_for() {
    let _serial = serial();
    let mut t = tree();
    // 空 Pane ⇒ 幂等：答同一个号，**不动水位**（下一枚铸出来的是 1，不是 2）。
    assert_eq!(t.part(Where::Root, name("dev")), Ok(EntryId::new(0)));
    assert_eq!(t.part(Where::Root, name("dev")), Ok(EntryId::new(0)));
    live(1);
    assert_eq!(
        t.land(Where::At(EntryId::new(0)), name("uart0"), tok(1)),
        Ok(EntryId::new(1))
    );
    // **非空** Pane ⇒ 还是幂等：答它那个号，里面那条一根毫毛没动。
    assert_eq!(t.part(Where::Root, name("dev")), Ok(EntryId::new(0)));
    assert_eq!(names(&t, &path(&["dev"])), Ok(std::vec![name("uart0")]));
    assert_eq!(look(&mut t, &path(&["dev", "uart0"])), Ok(Some(tok(1))));
}

#[test]
fn rebinding_takes_the_name_over_and_lets_the_old_one_go() {
    let _serial = serial();
    let mut t = tree();
    // 空 Pane ⇒ 换绑成一枚 `Tile`（没有旧句柄要放下）：**号不动**。

    assert_eq!(t.part(Where::Root, name("dev")), Ok(EntryId::new(0)));
    live(1);
    assert_eq!(
        t.land(Where::Root, name("dev"), tok(1)),
        Ok(EntryId::new(0))
    );
    assert_eq!(look(&mut t, &path(&["dev"])), Ok(Some(tok(1))));

    // 一枚 `Tile` ⇒ 换绑：旧的那一枚放下，号还是那一枚。

    live(2);
    assert_eq!(
        t.land(Where::Root, name("dev"), tok(2)),
        Ok(EntryId::new(0))
    );
    assert_eq!(look(&mut t, &path(&["dev"])), Ok(Some(tok(2))));
    assert!(unshipped(1) && !unshipped(2));

    // 一枚 `Tile` ⇒ 分成一块空 `Pane`：旧的那一枚也放下，号照旧。

    assert_eq!(t.part(Where::Root, name("dev")), Ok(EntryId::new(0)));
    assert!(unshipped(2));
    assert_eq!(names(&t, &[]), Ok(std::vec![name("dev")]));
    assert_eq!(names(&t, &path(&["dev"])), Ok(std::vec![]));
    assert_eq!(look(&mut t, &path(&["dev"])), Err(Fail::NotATile));
}

#[test]
fn opens_answers_the_opener_and_never_hands_the_pie_out() {
    let _serial = serial();
    let mut t = tree();
    live(1);
    assert_eq!(
        t.land(Where::Root, name("door"), tok(1)),
        Ok(EntryId::new(0))
    );
    // 开者那一格：**与授与人那一格分得开**（两枚戳子同型 ⇒ 偏置就是这一格的判据）。
    assert_eq!(
        t.opens(EntryId::new(0)),
        Ok(TaskId::new(1 + OPENED_BIAS)),
        "答的是开者，不是授与人"
    );
    // **什么都不交出去**：读它一遍之后那一格照旧在、那一枚照旧没被放下。
    assert!(!unshipped(1));
    assert_eq!(t.opens(EntryId::new(0)), Ok(TaskId::new(1 + OPENED_BIAS)));
    assert_eq!(
        look(&mut t, &path(&["door"])),
        Ok(Some(tok(1))),
        "句柄只有 find 交"
    );
}

#[test]
fn opens_knows_a_pane_a_tombstone_and_a_sealed_door() {
    let _serial = serial();
    let mut t = tree();
    // 一块 `Pane`：没有开者这一说。
    assert_eq!(t.part(Where::Root, name("dev")), Ok(EntryId::new(0)));
    assert_eq!(t.opens(EntryId::new(0)), Err(Fail::NotATile));
    // 墓碑：剪掉之后那条号**不重用**，一枚旧号永远答 `Unknown`。
    live(1);
    assert_eq!(
        t.land(Where::At(EntryId::new(0)), name("uart0"), tok(1)),
        Ok(EntryId::new(1))
    );
    assert_eq!(t.trim(EntryId::new(1)), Ok(()));
    assert_eq!(t.opens(EntryId::new(1)), Err(Fail::Unknown));
    // 从没铸过的号：与墓碑同一个码。
    assert_eq!(t.opens(EntryId::new(9)), Err(Fail::Unknown));
    // 是枚砖，但开者那扇门封印了 ⇒ `Dead`（**三因一码**，与 `find` 同一格）。
    assert_eq!(
        t.land(Where::At(EntryId::new(0)), name("uart1"), tok(2)),
        Ok(EntryId::new(2))
    );
    live(2);
    assert_eq!(t.opens(EntryId::new(2)), Ok(TaskId::new(1 + OPENED_BIAS)));
    gone(2);
    assert_eq!(t.opens(EntryId::new(2)), Err(Fail::Dead));
    // **`opens` 不动树**：剔死是 `find` 那一路上的事，故那一格照旧在。
    assert_eq!(t.name(EntryId::new(2)), Ok(name("uart1")));
}

#[test]
fn a_part_over_a_tile_takes_the_name_over_and_keeps_the_number() {
    // **那个窄口子**（`Operator::part` 的那条照实记）：`part` 碰到一枚 `Tile` 会
    // **静默**把它顶成一块 `Pane`——号不动、旧的那一枚被放下。今天 `/sys`、`/device` 一直是
    // `Pane`，故这条路上没有客人；这一条判据只把**现状**钉住（谁要改它，先看这里）。
    let _serial = serial();
    let mut t = tree();
    live(1);
    assert_eq!(
        t.land(Where::Root, name("uart0"), tok(1)),
        Ok(EntryId::new(0))
    );
    assert_eq!(
        t.part(Where::Root, name("uart0")),
        Ok(EntryId::new(0)),
        "换绑不动号"
    );
    assert!(unshipped(1), "旧的那一枚被放下了（不加这一格就漏在树里）");
    assert_eq!(
        t.list(Where::At(EntryId::new(0))).map(Iterator::count),
        Ok(0),
        "那一名下现在是一块**空** Pane"
    );
    assert_eq!(t.name(EntryId::new(0)), Ok(name("uart0")));
    // 顶完之后它不是砖了：寻它答 `NotATile`，而它下面能再立一格。
    assert_eq!(look(&mut t, &path(&["uart0"])), Err(Fail::NotATile));
    assert_eq!(
        t.land(Where::At(EntryId::new(0)), name("inner"), tok(2)),
        Ok(EntryId::new(1))
    );
}

#[test]
fn the_same_pie_under_two_names_is_two_independent_entries() {
    // **别名**：同一枚 Pie 挂两个名 = 两条**独立**条目。
    // 树是"名字 → 一枚句柄"的目录，**不查重**（`land` 的判据里没有"这一枚已经挂过了"）。
    // 这一条只把**现状**钉住：两条各自可寻、各自可剪，剪一条不动另一条。
    let _serial = serial();
    let mut t = tree();
    live(1);
    assert_eq!(t.land(Where::Root, name("a"), tok(1)), Ok(EntryId::new(0)));
    assert_eq!(
        t.land(Where::Root, name("b"), tok(1)),
        Ok(EntryId::new(1)),
        "另一条条目（号不同）"
    );
    // 两条指着**同一扇门**：开者那一问答同一个号（`opens` 不看名字，看那一枚句柄）。
    assert_eq!(
        t.opens(EntryId::new(0)),
        t.opens(EntryId::new(1)),
        "别名：两条指的是同一枚"
    );
    assert_eq!(look(&mut t, &path(&["a"])), Ok(Some(tok(1))));
    assert_eq!(look(&mut t, &path(&["b"])), Ok(Some(tok(1))));
    // 剪一条：那一份被放下，另一条照旧（各自持着自己那一份）。
    assert_eq!(t.trim(EntryId::new(0)), Ok(()));
    assert!(unshipped(1));
    assert_eq!(names(&t, &[]), Ok(std::vec![name("b")]));
    assert_eq!(look(&mut t, &path(&["b"])), Ok(Some(tok(1))), "另一条照旧");
}

#[test]
fn a_dead_tile_is_swept_on_the_read_path() {
    let _serial = serial();
    let mut t = tree();
    live(1);
    assert_eq!(
        t.land(Where::Root, name("log"), tok(1)),
        Ok(EntryId::new(0))
    );
    gone(1);
    // 寻之前先译号：译号**不过问死活**（与 `list` 一样），剔死落在 `find` 那一格上。
    assert_eq!(t.seek(&path(&["log"])), Ok(EntryId::new(0)));
    assert_eq!(look(&mut t, &path(&["log"])), Err(Fail::Dead));
    assert!(unshipped(1));
    assert_eq!(look(&mut t, &path(&["log"])), Err(Fail::Unknown));
    assert_eq!(names(&t, &[]), Ok(std::vec![]));
}

#[test]
fn a_dead_tile_inside_a_pane_leaves_the_pane_alone() {
    let _serial = serial();
    let mut t = tree();
    assert_eq!(t.part(Where::Root, name("dev")), Ok(EntryId::new(0)));
    live(1);
    assert_eq!(
        t.land(Where::At(EntryId::new(0)), name("uart0"), tok(1)),
        Ok(EntryId::new(1))
    );
    gone(1);
    assert_eq!(look(&mut t, &path(&["dev", "uart0"])), Err(Fail::Dead));
    assert_eq!(names(&t, &path(&["dev"])), Ok(std::vec![]));
    assert_eq!(names(&t, &[]), Ok(std::vec![name("dev")]));
    assert_eq!(t.trim(EntryId::new(0)), Ok(()), "空下来了，剪得掉");
}

#[test]
fn trimming_lets_go_of_the_tile_and_keeps_empty_panes() {
    let _serial = serial();
    let mut t = tree();
    live(1);
    assert_eq!(
        t.land(Where::Root, name("log"), tok(1)),
        Ok(EntryId::new(0))
    );
    assert_eq!(t.trim(EntryId::new(0)), Ok(()));
    assert!(unshipped(1));
    assert_eq!(look(&mut t, &path(&["log"])), Err(Fail::Unknown));

    // 剪掉不回收号：下一枚铸出来的是 1。
    assert_eq!(t.part(Where::Root, name("dev")), Ok(EntryId::new(1)));
    assert_eq!(t.trim(EntryId::new(1)), Ok(()));
    assert_eq!(names(&t, &[]), Ok(std::vec![]));
}

#[test]
fn the_tree_has_a_bottom() {
    let _serial = serial();
    let mut t = tree();
    let mut texts: Vec<String> = Vec::new();
    for i in 0..Operator::PANE_CAP {
        live(i);
        let text = std::format!("n{i}");
        assert_eq!(
            t.land(Where::Root, name(text.as_str()), tok(i)),
            Ok(EntryId::new(i)),
            "第 {i} 条该落得上，号就是 {i}"
        );
        texts.push(text);
    }
    assert_eq!(names(&t, &[]).map(|v| v.len()), Ok(Operator::PANE_CAP));

    // 令牌是真的、那一层是满的 ⇒ 这才是 Full
    live(Operator::PANE_CAP);
    assert_eq!(
        t.land(Where::Root, name("n17"), tok(Operator::PANE_CAP)),
        Err(Fail::Full)
    );
    assert_eq!(t.part(Where::Root, name("n18")), Err(Fail::Full));

    // 路太深：**今天只有 `seek` 还有"路"**——别的原语收号，没有路的深浅可言。
    let deep: Vec<Name> = (0..=Operator::ROAD_MAX)
        .map(|i| name(&std::format!("d{i}")))
        .collect();
    assert_eq!(t.seek(&deep), Err(Fail::Full));
}

#[test]
fn a_pane_that_cannot_be_grown_answers_full() {
    // **同一句话，三处一个纪律**：`Desk::admit`（`crates/contract/src/system/desk.rs`）与 `Ledger::land`
    // （`crates/contract/src/system/operator/core/ledger.rs`）都是 `try_reserve → Full`，而树这一处原来
    // 只有**条数**那道闸（`PANE_CAP`）——分配失败走的是 `handle_alloc_error`，客人连一句答话
    // 都收不到（不是 `Full`，是整机 abort）。
    //
    // 照实记：这一条判据**只有把分配真的打掉**才量得到，故本台的分配器是可关的（见文件头那一
    // 节）。它同时钉住"失败**没有副作用**"：那一格没被占、号水位也没往前走。
    let _serial = serial();
    let mut t = tree();
    live(0);

    NO_ROOM.with(|flag| flag.set(true));
    let landed = t.land(Where::Root, name("n0"), tok(0));
    let parted = t.part(Where::Root, name("n1"));
    NO_ROOM.with(|flag| flag.set(false));

    assert_eq!(landed, Err(Fail::Full), "备不下要如实报 Full，不是 abort");
    assert_eq!(parted, Err(Fail::Full));
    assert_eq!(t.seek(&[name("n0")]), Err(Fail::Unknown), "没落上");
    assert_eq!(
        t.list(Where::Root).map(|ids| ids.count()),
        Ok(0),
        "那一层还是空的"
    );
    // 旗一撤，同一次调用就走通了——说明刚才那两下**什么都没留下**（号也没被吃掉）。
    assert_eq!(t.land(Where::Root, name("n0"), tok(0)), Ok(EntryId::new(0)));
}

#[test]
fn a_deep_chain_does_not_need_the_call_stack() {
    // **这一条钉的是"深度不吃调用栈"**。照实记：老一版四条助手（`look` / `holds` / `take` /
    // `put_in`）按深度递归，而一台域的栈只有 `TASK_STACK_SIZE` = 16 KiB——**真机读数**
    // （打这一枪的那台探针 `prog-probe-deep` 已随 `fair` 一景按用户裁定删掉，读数留着）：
    // 它让持树者自己死在第 117 层（`user fault killed: tid=3`），命名空间整个消失。
    //
    // 故这里**故意把测试线程的栈压到 64 KiB**，再建一条 500 层的链：老一版在 500 层上要几十上百
    // KB，**当场 SIGSEGV**（这台的机制与 `a_pane_that_cannot_be_grown_answers_full` 同一手法：
    // 把判据做成"资源真的不够时也活得下来"，而不是"跑得完"）。
    //
    // 深链只走 `part` / `name` / `list` / `trim`：四条都点得到（老版的）那四个递归助手，而都不碰
    // 活性表（故不必把 `TABLE` 撑到几百位）。
    let _serial = serial();
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            const DEEP: u32 = 500;
            let mut t = tree();
            let mut at = Where::Root;
            let mut chain: Vec<EntryId> = Vec::new();
            for i in 0..DEEP {
                let text = std::format!("d{i}");
                let id = t.part(at, name(text.as_str())).expect("分得上");
                chain.push(id);
                at = Where::At(id);
            }
            // 最底那一格：名字答得出、里面是空的（故剪得动）。
            let deepest = *chain.last().expect("非空");
            assert_eq!(t.name(deepest), Ok(name("d499")));
            assert_eq!(t.list(Where::At(deepest)).map(|ids| ids.count()), Ok(0));

            // 从最底往上一层一层剪回去：每一手都要走到那条链的深处。
            for (i, id) in chain.iter().enumerate().rev() {
                assert_eq!(t.trim(*id), Ok(()), "剪第 {i} 层");
                // 剪过的那一号**不对外说话**（墓碑与"从没铸过"长得一样）。
                assert_eq!(t.name(*id), Err(Fail::Unknown), "第 {i} 层剪过之后");
            }
            assert_eq!(names(&t, &[]).map(|v| v.len()), Ok(0), "根那一层也空了");
        })
        .expect("起得来线程")
        .join()
        .expect("这条链不该把栈打穿");
}

#[test]
fn the_root_is_not_an_entry() {
    let _serial = serial();
    let mut t = tree();
    live(1);
    // 根**不是**谁条目里的一条：`Where::Root` 是合法坐标（它说的是"根"），而"某一号"那一路
    // 递不进根——`find` / `trim` / `name` 的形参就是号，收不下它（类型义务）。
    assert_eq!(t.list(Where::Root).map(|ids| ids.count()), Ok(0));
    // **根没有号**：译不出来（对照 `list(Where::Root)`：列根那一层不需要根有号）。
    assert_eq!(t.seek(&[]), Err(Fail::Unknown));
    assert_eq!(
        t.land(Where::At(EntryId::new(9)), name("x"), tok(1)),
        Err(Fail::Unknown)
    );
}

#[test]
fn seeking_translates_a_road_into_the_id_of_that_cell() {
    let _serial = serial();
    let mut t = tree();
    assert_eq!(t.part(Where::Root, name("dev")), Ok(EntryId::new(0)));
    // 走到哪儿答**哪一格自己的号**：`dev` 是第 0 条铸出来的。
    assert_eq!(t.seek(&path(&["dev"])), Ok(EntryId::new(0)));
    live(1);
    assert_eq!(
        t.land(Where::At(EntryId::new(0)), name("uart0"), tok(1)),
        Ok(EntryId::new(1))
    );
    assert_eq!(t.seek(&path(&["dev", "uart0"])), Ok(EntryId::new(1)));
    // 最后一段是一枚**砖**也行（`list` 那一格走到同一个地方就答 `NotAPane` 了）。
    assert_eq!(names(&t, &path(&["dev", "uart0"])), Err(Fail::NotAPane));
    // 中途是一枚砖 ⇒ 走不过去（与 `list` 同一条走法）。
    assert_eq!(t.seek(&path(&["dev", "uart0", "x"])), Err(Fail::NotAPane));

    // **换绑不动号**：拿同一枚号回头问，还是那一格、还是那个名字。
    live(2);
    assert_eq!(
        t.land(Where::At(EntryId::new(0)), name("uart0"), tok(2)),
        Ok(EntryId::new(1))
    );
    assert_eq!(t.seek(&path(&["dev", "uart0"])), Ok(EntryId::new(1)));
    assert_eq!(t.name(EntryId::new(1)), Ok(name("uart0")));

    // 缺一段 ⇒ `Unknown`；空路是根 ⇒ 也是 `Unknown`。
    assert_eq!(t.seek(&path(&["dev", "nope"])), Err(Fail::Unknown));
    assert_eq!(t.seek(&path(&["device", "uart0"])), Err(Fail::Unknown));
    assert_eq!(t.seek(&[]), Err(Fail::Unknown));
    assert_eq!(names(&t, &[]), Ok(std::vec![name("dev")]));
}

#[test]
fn zero_is_a_real_cell() {
    let _serial = serial();
    let mut t = tree();
    // 第一条铸出来的号就是 0——**不是"没有"**：根没有号，0 是真格子。
    assert_eq!(t.part(Where::Root, name("sys")), Ok(EntryId::new(0)));
    let ids: Vec<EntryId> = t.list(Where::Root).unwrap().collect();
    assert_eq!(ids, std::vec![EntryId::new(0)]);
    assert_eq!(t.name(EntryId::new(0)), Ok(name("sys")));
    assert_eq!(
        t.name(EntryId::new(1)),
        Err(Fail::Unknown),
        "没铸过 ⇒ Unknown"
    );
}

#[test]
fn rebinding_keeps_the_number_and_trimming_retires_it() {
    let _serial = serial();
    let mut t = tree();
    live(1);
    assert_eq!(
        t.land(Where::Root, name("log"), tok(1)),
        Ok(EntryId::new(0))
    );
    let id = t.list(Where::Root).unwrap().next().unwrap();
    live(2);
    assert_eq!(
        t.land(Where::Root, name("log"), tok(2)),
        Ok(id),
        "换绑不动号"
    );
    assert_eq!(t.list(Where::Root).unwrap().next(), Some(id), "换绑不动号");
    assert_eq!(t.name(id), Ok(name("log")));
    assert_eq!(t.trim(id), Ok(()));
    assert_eq!(t.name(id), Err(Fail::Unknown), "剪掉 ⇒ 号失效");
}
