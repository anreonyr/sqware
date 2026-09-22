//! operator 核心的门（**宿主台**）—— 树与七条原语的规矩，在宿主上真跑一遍。
//!
//! # 这批判据为什么住在这里
//!
//! `protocol` 是 `[lib] test = false`（目标 riscv64gc-unknown-none-elf 上编不出 libtest），
//! 故 `cargo check --workspace --all-targets` 与 `build --release` **都不编**它的
//! `#[cfg(test)]`；而**链接 `protocol` 去测**也走不通——它依赖 `runtime`，`runtime` 里那两处
//! riscv 内联汇编在宿主编译器上编不出来（见本 crate 的 `Cargo.toml`，那里记着实测读数）。
//!
//! 故这一台与 `crates/alloc-probe` 同路：宿主 crate、只依赖 `env`，把
//! `crates/protocol/src/operator/core.rs` **逐字未改**地编进测试靶里。
//!
//! 用 `#[path]` 而不是 `include!`——**照实记**：`include!` 那一版第一跑就红，
//! `error[E0753]: expected outer doc comment` 报了六次：被编进来的文件以 `//!` 开头
//! （模块内文档），而宏展开出来的内部属性落不到"模块体的开头"那个位置上。
//! `#[path]` 把它当一个真正的模块文件读，`//!` 就正正经经是那个模块的文档。
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

/// 树的正文（就是 `crates/protocol/src/operator/core.rs` 那一份，逐字未改）。
#[path = "../../protocol/src/operator/core.rs"]
mod operator;

use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use env::{Name, PieToken, TaskId};

use crate::operator::{EntryId, Fail, Operator, Where};

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
    Operator::new(fake_vested_by, fake_unship)
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
    assert_eq!(t.land(Where::Root, name("uart0"), tok(1)), Ok(EntryId::new(0)));
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
    assert_eq!(t.part(Where::At(EntryId::new(7)), name("sub")), Err(Fail::Unknown));
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
    assert_eq!(t.land(Where::Root, name("log"), tok(1)), Ok(EntryId::new(0)));
    live(2);
    // 拿一枚 `Tile` 当容器 ⇒ 走不进去（落 / 分 / 列 三条都走这一格）。
    assert_eq!(
        t.land(Where::At(EntryId::new(0)), name("x"), tok(2)),
        Err(Fail::NotAPane)
    );
    assert_eq!(t.part(Where::At(EntryId::new(0)), name("x")), Err(Fail::NotAPane));
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
    assert_eq!(t.land(Where::Root, name("log"), tok(1)), Ok(EntryId::new(0)));
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
    assert_eq!(t.land(Where::Root, name("dev"), tok(2)), Err(Fail::NonEmpty));
    assert_eq!(t.trim(EntryId::new(0)), Err(Fail::NonEmpty), "非空 Pane 剪不动");
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
    assert_eq!(t.land(Where::Root, name("dev"), tok(1)), Ok(EntryId::new(0)));
    assert_eq!(look(&mut t, &path(&["dev"])), Ok(Some(tok(1))));

    // 一枚 `Tile` ⇒ 换绑：旧的那一枚放下，号还是那一枚。

    live(2);
    assert_eq!(t.land(Where::Root, name("dev"), tok(2)), Ok(EntryId::new(0)));
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
fn a_dead_tile_is_swept_on_the_read_path() {
    let _serial = serial();
    let mut t = tree();
    live(1);
    assert_eq!(t.land(Where::Root, name("log"), tok(1)), Ok(EntryId::new(0)));
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
    assert_eq!(t.land(Where::Root, name("log"), tok(1)), Ok(EntryId::new(0)));
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
    assert_eq!(t.land(Where::Root, name("log"), tok(1)), Ok(EntryId::new(0)));
    let id = t.list(Where::Root).unwrap().next().unwrap();
    live(2);
    assert_eq!(t.land(Where::Root, name("log"), tok(2)), Ok(id), "换绑不动号");
    assert_eq!(t.list(Where::Root).unwrap().next(), Some(id), "换绑不动号");
    assert_eq!(t.name(id), Ok(name("log")));
    assert_eq!(t.trim(id), Ok(()));
    assert_eq!(t.name(id), Err(Fail::Unknown), "剪掉 ⇒ 号失效");
}
