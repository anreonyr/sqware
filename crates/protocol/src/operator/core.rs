//! operator 的核心 —— **树、五条原语（落 / 分 / 寻 / 剪 / 列）、失败域**。
//!
//! 本文件**不碰内核**：判据只有一条可机械检查的纪律——
//!
//! > `core.rs` 里不出现 `runtime::`。
//!
//! 两个外部事实是**注入**的：[`Probe`]（那一枚 Pie 还答得出吗）与 [`Free`]（把我这一份放下）。
//! 于是喂两个假闭包就能把这棵树与五条原语的规矩推理干净，换载体不必重写。

use alloc::vec::Vec;

use env::{Name, PieToken, TaskId};

// ── 结构 ────────────────────────────────────────────────────

/// 一条条目：**名字 + 去处**。
///
/// 名字只是**一段**（`Name`：定长 32 字节、构造即校验），不是整条路。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    name: Name,
    node: Node,
}

/// 去处：一块 [`Node::Pane`]（窗格，还能往里走）或一枚 [`Node::Tile`]（砖，到头了）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// 一块 Pane（窗格）：里面是条目。**还能往里走**。
    Pane(Vec<Entry>),
    /// 一枚 Tile（砖）：到头了，就是内核给的那一枚句柄。
    Tile(PieToken),
}

impl Entry {
    /// 这一条叫什么（一段）。
    pub fn name(&self) -> Name {
        self.name
    }

    /// 它去哪儿。
    pub fn node(&self) -> &Node {
        &self.node
    }
}

/// 五条原语会失败在哪一格。**一格对应一个不同的下一步**。
///
/// **没有"名字已被占"那一格**：同名接手一枚 `Tile`、或一块**空的** `Pane`，都是换绑
/// （见 [`Operator::land`] / [`Operator::part`]）；而 owner 归 Principal，Operator 分不出
/// "自己 / 别人"，所以"已占即拒"在这里无处落脚。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 路上没有这一段 ⇒ 换个名字，或者先把中间层分出来。
    ///
    /// **空路也走这一格**：空路是"根"本身，而根不是谁条目里的一条——落 / 分 / 剪对根都动手不了。
    Unknown,
    /// 那块 `Pane` 里还有东西 ⇒ 先清空。
    NonEmpty,
    /// 寻到头是一块 `Pane`，不是一枚 `Tile` ⇒ 改用列，或者再往下走一段。
    NotATile,
    /// 那一段不是一块 `Pane`（是一枚 `Tile`）⇒ 走不过去；列的时候则说明"那是枚 `Tile`，没什么可列"。
    NotAPane,
    /// 一块 `Pane` 装不下，或者一条路太长 ⇒ 拆层 / 扩容量。
    Full,
    /// 那枚 Pie 后面的人没了（探不到）⇒ 重落 / 重寻。**剔掉那一条的同时**答这一格。
    Dead,
}

/// **活性**：那一枚 Pie 还答得出吗？答不出（`None`）= 它后面的人没了。
///
/// 与 `board` 同一格（`Probe` 的形状照旧）：本正文没有 owner，故这里只取"答得出吗"，
/// 答出来的 `TaskId` 用不到。
pub type Probe = fn(PieToken) -> Option<TaskId>;

/// **放下**：把我这一份自释。剪掉或换掉一枚 `Tile` 时用它——不加这一格，那一枚句柄就漏在树里。
pub type Free = fn(PieToken) -> Result<(), ()>;

// ── 树 ──────────────────────────────────────────────────────

/// 一棵命名树：**一个 Operator 管着所有条目**。
///
/// 根 = `root` 那一叠条目；**空路就是根**（[`Operator::list`] 列的就是它那一层）。
pub struct Operator {
    root: Vec<Entry>,
    probe: Probe,
    free: Free,
}

impl Operator {
    /// 一块 `Pane` 里最多几条。条数是策略，容器要有界。
    pub const PANE_CAP: usize = 16;
    /// 一条路最多几段。深也是策略。
    pub const PATH_MAX: usize = 8;

    /// 立一棵树：两个注入的机制事实跟着树走——它们对每一条同值，故不必逐个作参数传。
    pub const fn new(probe: Probe, free: Free) -> Operator {
        Operator {
            root: Vec::new(),
            probe,
            free,
        }
    }

    /// **落**：把一枚 Pie 落到一条路上（放一枚 `Tile`）。
    ///
    /// 四条判据，一条不多：
    ///
    /// - 路非空、有界（空路 ⇒ [`Fail::Unknown`]；超深 ⇒ [`Fail::Full`]）；
    /// - 路上除最后一段外**都得是 `Pane` 且存在**（缺一段 ⇒ [`Fail::Unknown`]；
    ///   那一段是一枚 `Tile` ⇒ [`Fail::NotAPane`]）；
    /// - 最后一段空着 ⇒ 落上；
    /// - 最后一段已经占着 ⇒ **换绑**：旧的那一枚放下（[`Free`]）——除非它是一块**非空** `Pane`
    ///   （⇒ [`Fail::NonEmpty`]：要动它先清空）。
    pub fn land(&mut self, path: &[Name], pie: PieToken) -> Result<(), Fail> {
        Self::checked(path)?;
        let free = self.free;
        let last = path[path.len() - 1];
        let level = self.level_mut(path)?;
        match level.iter().position(|e| e.name == last) {
            Some(at) => {
                // 已占：只有"一枚 `Tile`"与"空 `Pane`"可以换绑；前者那一枚要放下。

                let old = match &level[at].node {
                    Node::Tile(old) => Some(*old),
                    Node::Pane(inner) if inner.is_empty() => None,
                    Node::Pane(_) => return Err(Fail::NonEmpty),
                };
                level[at].node = Node::Tile(pie);
                if let Some(old) = old {
                    let _ = free(old);
                }
                Ok(())
            }
            None => {
                if level.len() >= Self::PANE_CAP {
                    return Err(Fail::Full);
                }
                level.push(Entry {
                    name: last,
                    node: Node::Tile(pie),
                });
                Ok(())
            }
        }
    }

    /// **分**：在一条路上分出一块**空的** `Pane`（开一块窗格）。
    ///
    /// 门槛与 [`Operator::land`] 同：路非空、有界，中间那几层都得是 `Pane` 且存在。
    /// 最后一段：空着 ⇒ 放一块空 `Pane`；已是 `Tile` ⇒ 换掉它（旧的那一枚放下）；
    /// 已是**空** `Pane` ⇒ 无事；已是**非空** `Pane` ⇒ [`Fail::NonEmpty`]。
    pub fn part(&mut self, path: &[Name]) -> Result<(), Fail> {
        Self::checked(path)?;
        let free = self.free;
        let last = path[path.len() - 1];
        let level = self.level_mut(path)?;
        match level.iter().position(|e| e.name == last) {
            Some(at) => {
                let old = match &level[at].node {
                    Node::Pane(inner) if inner.is_empty() => return Ok(()),
                    Node::Pane(_) => return Err(Fail::NonEmpty),
                    Node::Tile(old) => Some(*old),
                };
                level[at].node = Node::Pane(Vec::new());
                if let Some(old) = old {
                    let _ = free(old);
                }
                Ok(())
            }
            None => {
                if level.len() >= Self::PANE_CAP {
                    return Err(Fail::Full);
                }
                level.push(Entry {
                    name: last,
                    node: Node::Pane(Vec::new()),
                });
                Ok(())
            }
        }
    }

    /// **寻**：走到头，把那一枚 Pie 交出去。
    ///
    /// - 走不动（中间那一段是一枚 `Tile`）⇒ [`Fail::NotAPane`]；缺一段 ⇒ [`Fail::Unknown`]；
    /// - 到头是一块 `Pane` ⇒ [`Fail::NotATile`]（**空路也是这一格**：根是一块 `Pane`）；
    /// - 到头是一枚 `Tile`：**先探一次**（[`Probe`]）——答不出 ⇒ 当场剔掉那一条、放下那一份
    ///   （[`Free`]），答 [`Fail::Dead`]；答得出 ⇒ 交给 `give`。
    ///
    /// `give` 是"交出去"那一手（适配层在这里把 Pie 授给调用方，核心因此不碰内核）。
    pub fn find(&mut self, path: &[Name], mut give: impl FnMut(PieToken)) -> Result<(), Fail> {
        if path.len() > Self::PATH_MAX {
            return Err(Fail::Full);
        }
        if path.is_empty() {
            return Err(Fail::NotATile);
        }
        let probe = self.probe;
        let free = self.free;
        let last = path[path.len() - 1];
        let level = self.level_mut(path)?;
        let at = level
            .iter()
            .position(|e| e.name == last)
            .ok_or(Fail::Unknown)?;
        let pie = match &level[at].node {
            Node::Pane(_) => return Err(Fail::NotATile),
            Node::Tile(pie) => *pie,
        };
        if probe(pie).is_none() {
            level.remove(at);
            let _ = free(pie);
            return Err(Fail::Dead);
        }
        give(pie);
        Ok(())
    }

    /// **剪**：把路上那一条剪掉。
    ///
    /// 那一条得存在；是 `Pane` 的话**必须空着**（否则 [`Fail::NonEmpty`]）。
    /// 剪掉一枚 `Tile` 时那一枚放下（[`Free`]）——它是资源实体的一份引用，不放下就漏水。
    pub fn trim(&mut self, path: &[Name]) -> Result<(), Fail> {
        Self::checked(path)?;
        let free = self.free;
        let last = path[path.len() - 1];
        let level = self.level_mut(path)?;
        let at = level
            .iter()
            .position(|e| e.name == last)
            .ok_or(Fail::Unknown)?;
        let dropped = match &level[at].node {
            Node::Pane(inner) if inner.is_empty() => None,
            Node::Pane(_) => return Err(Fail::NonEmpty),
            Node::Tile(pie) => Some(*pie),
        };
        level.remove(at);
        if let Some(pie) = dropped {
            let _ = free(pie);
        }
        Ok(())
    }

    /// **列**：看一块 `Pane` 里有哪些名字。
    ///
    /// **空路 = 根**（列根那一层）。缺一段 ⇒ [`Fail::Unknown`]；走不动或到头是一枚 `Tile`
    /// ⇒ [`Fail::NotAPane`]。**不过问死活**：剔死是 [`Operator::find`] 那一路上的事。
    pub fn list(&self, path: &[Name]) -> Result<impl Iterator<Item = Name> + '_, Fail> {
        if path.len() > Self::PATH_MAX {
            return Err(Fail::Full);
        }
        let mut level = &self.root;
        for step in path {
            let entry = level
                .iter()
                .find(|e| e.name == *step)
                .ok_or(Fail::Unknown)?;
            level = match &entry.node {
                Node::Pane(inner) => inner,
                Node::Tile(_) => return Err(Fail::NotAPane),
            };
        }
        Ok(level.iter().map(|e| e.name))
    }

    // ── 走路 ────────────────────────────────────────────────

    /// 落 / 分 / 剪共同的两条门槛：**路非空**（空路是根本身）、**路不超深**。
    fn checked(path: &[Name]) -> Result<(), Fail> {
        if path.is_empty() {
            return Err(Fail::Unknown);
        }
        if path.len() > Self::PATH_MAX {
            return Err(Fail::Full);
        }
        Ok(())
    }

    /// 走到 `path` 的**上一层**（最后一段所在的那一叠）。
    ///
    /// **调用方先过 [`Operator::checked`]**（或自己挡住空路）：这里按 `path[..len - 1]` 切，
    /// 空路上切不出来。
    fn level_mut(&mut self, path: &[Name]) -> Result<&mut Vec<Entry>, Fail> {
        let mut level = &mut self.root;
        for step in &path[..path.len() - 1] {
            let at = level
                .iter()
                .position(|e| e.name == *step)
                .ok_or(Fail::Unknown)?;
            level = match &mut level[at].node {
                Node::Pane(inner) => inner,
                Node::Tile(_) => return Err(Fail::NotAPane),
            };
        }
        Ok(level)
    }
}

// ── 宿主测试（不进 riscv 构建）────────────────────────────────
//
// 跑法与实测有效的那条路写在 `crates/protocol/Cargo.toml` 的 `[lib]` 注记里
// （本 crate 依赖 `runtime`，而它含 RISC-V 汇编 ⇒ 临时宿主驱动 `include!` 本文件）。
//
// 两个注入点就是全部外部依赖，故喂两张假表即可把树与五条原语的规矩推理干净。

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// 假表是**进程级**的（`Probe` / `Free` 是函数指针，捕不了环境），故测试彼此串行。
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

    fn fake_probe(entry: PieToken) -> Option<TaskId> {
        match entry.get() {
            at if at < TABLE.len() => match TABLE[at].load(Ordering::Relaxed) {
                0 => None,
                who => Some(TaskId::new(who)),
            },
            _ => None,
        }
    }

    /// 记下"这一枚被放下了"。假表有 `PANE_CAP + 1` 位，越界的令牌不记。
    fn fake_free(entry: PieToken) -> Result<(), ()> {
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
        Operator::new(fake_probe, fake_free)
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

    fn freed(entry: usize) -> bool {
        FREED.load(Ordering::Relaxed) & (1usize << entry) != 0
    }

    fn name(text: &str) -> Name {
        Name::new(text).unwrap()
    }

    fn path(texts: &[&str]) -> Vec<Name> {
        texts.iter().map(|t| name(t)).collect()
    }

    /// 列一层，收成 `Vec`（免得到处写 `.collect::<Vec<_>>()`）。
    fn names(op: &Operator, at: &[Name]) -> Result<Vec<Name>, Fail> {
        op.list(at).map(|it| it.collect())
    }

    /// 寻一条路，把交出来的那一枚记下来。
    fn look(op: &mut Operator, at: &[Name]) -> Result<Option<PieToken>, Fail> {
        let mut got = None;
        op.find(at, |pie| got = Some(pie)).map(|()| got)
    }

    #[test]
    fn a_tile_lands_and_find_hands_it_back() {
        let _serial = serial();
        let mut t = tree();
        live(1);
        assert_eq!(t.land(&path(&["uart0"]), tok(1)), Ok(()));
        assert_eq!(look(&mut t, &path(&["uart0"])), Ok(Some(tok(1))));
        assert_eq!(names(&t, &[]), Ok(std::vec![name("uart0")]));
    }

    #[test]
    fn a_pane_opens_a_second_level() {
        let _serial = serial();
        let mut t = tree();
        assert_eq!(t.part(&path(&["dev"])), Ok(()));
        live(1);
        assert_eq!(t.land(&path(&["dev", "uart0"]), tok(1)), Ok(()));
        assert_eq!(look(&mut t, &path(&["dev", "uart0"])), Ok(Some(tok(1))));
        assert_eq!(names(&t, &path(&["dev"])), Ok(std::vec![name("uart0")]));
        assert_eq!(names(&t, &[]), Ok(std::vec![name("dev")]));
    }

    #[test]
    fn a_missing_segment_is_unknown() {
        let _serial = serial();
        let mut t = tree();
        live(1);
        assert_eq!(t.land(&path(&["dev", "uart0"]), tok(1)), Err(Fail::Unknown));
        assert_eq!(t.part(&path(&["dev", "sub"])), Err(Fail::Unknown));
        assert_eq!(t.trim(&path(&["dev", "uart0"])), Err(Fail::Unknown));
        assert_eq!(look(&mut t, &path(&["dev", "uart0"])), Err(Fail::Unknown));
        assert_eq!(names(&t, &path(&["dev", "uart0"])), Err(Fail::Unknown));
    }

    #[test]
    fn walking_through_a_tile_is_not_a_pane() {
        let _serial = serial();
        let mut t = tree();
        live(1);
        assert_eq!(t.land(&path(&["log"]), tok(1)), Ok(()));
        live(2);
        assert_eq!(t.land(&path(&["log", "x"]), tok(2)), Err(Fail::NotAPane));
        assert_eq!(t.part(&path(&["log", "x"])), Err(Fail::NotAPane));
        assert_eq!(t.trim(&path(&["log", "x"])), Err(Fail::NotAPane));
        assert_eq!(look(&mut t, &path(&["log", "x"])), Err(Fail::NotAPane));
        assert_eq!(names(&t, &path(&["log"])), Err(Fail::NotAPane));
        assert_eq!(names(&t, &path(&["log", "x"])), Err(Fail::NotAPane));
    }

    #[test]
    fn finding_a_pane_is_not_a_tile() {
        let _serial = serial();
        let mut t = tree();
        assert_eq!(t.part(&path(&["dev"])), Ok(()));
        assert_eq!(look(&mut t, &path(&["dev"])), Err(Fail::NotATile));
        assert_eq!(look(&mut t, &[]), Err(Fail::NotATile));
    }

    #[test]
    fn listing_a_tile_is_not_a_pane() {
        let _serial = serial();
        let mut t = tree();
        live(1);
        assert_eq!(t.land(&path(&["log"]), tok(1)), Ok(()));
        assert_eq!(names(&t, &path(&["log"])), Err(Fail::NotAPane));
    }

    #[test]
    fn a_pane_with_things_in_it_is_not_moved() {
        let _serial = serial();
        let mut t = tree();
        assert_eq!(t.part(&path(&["dev"])), Ok(()));
        live(1);
        assert_eq!(t.land(&path(&["dev", "uart0"]), tok(1)), Ok(()));
        live(2);
        assert_eq!(t.land(&path(&["dev"]), tok(2)), Err(Fail::NonEmpty));
        assert_eq!(t.part(&path(&["dev"])), Err(Fail::NonEmpty));
        assert_eq!(t.trim(&path(&["dev"])), Err(Fail::NonEmpty));
        assert!(!freed(1), "非空那块 Pane 一根毫毛都没动");
        assert_eq!(look(&mut t, &path(&["dev", "uart0"])), Ok(Some(tok(1))));
    }

    #[test]
    fn rebinding_takes_the_name_over_and_lets_the_old_one_go() {
        let _serial = serial();
        let mut t = tree();
        // 空 Pane ⇒ 换绑成一枚 `Tile`（没有旧句柄要放下）

        assert_eq!(t.part(&path(&["dev"])), Ok(()));
        live(1);
        assert_eq!(t.land(&path(&["dev"]), tok(1)), Ok(()));
        assert_eq!(look(&mut t, &path(&["dev"])), Ok(Some(tok(1))));

        // 一枚 `Tile` ⇒ 换绑：旧的那一枚放下

        live(2);
        assert_eq!(t.land(&path(&["dev"]), tok(2)), Ok(()));
        assert_eq!(look(&mut t, &path(&["dev"])), Ok(Some(tok(2))));
        assert!(freed(1) && !freed(2));

        // 一枚 `Tile` ⇒ 分成一块空 `Pane`：旧的那一枚也放下

        assert_eq!(t.part(&path(&["dev"])), Ok(()));
        assert!(freed(2));
        assert_eq!(names(&t, &[]), Ok(std::vec![name("dev")]));
        assert_eq!(names(&t, &path(&["dev"])), Ok(std::vec![]));
        assert_eq!(look(&mut t, &path(&["dev"])), Err(Fail::NotATile));
    }

    #[test]
    fn a_dead_tile_is_swept_on_the_read_path() {
        let _serial = serial();
        let mut t = tree();
        live(1);
        assert_eq!(t.land(&path(&["log"]), tok(1)), Ok(()));
        gone(1);
        assert_eq!(look(&mut t, &path(&["log"])), Err(Fail::Dead));
        assert!(freed(1));
        assert_eq!(look(&mut t, &path(&["log"])), Err(Fail::Unknown));
        assert_eq!(names(&t, &[]), Ok(std::vec![]));
    }

    #[test]
    fn a_dead_tile_inside_a_pane_leaves_the_pane_alone() {
        let _serial = serial();
        let mut t = tree();
        assert_eq!(t.part(&path(&["dev"])), Ok(()));
        live(1);
        assert_eq!(t.land(&path(&["dev", "uart0"]), tok(1)), Ok(()));
        gone(1);
        assert_eq!(look(&mut t, &path(&["dev", "uart0"])), Err(Fail::Dead));
        assert_eq!(names(&t, &path(&["dev"])), Ok(std::vec![]));
        assert_eq!(names(&t, &[]), Ok(std::vec![name("dev")]));
        assert_eq!(t.trim(&path(&["dev"])), Ok(()), "空下来了，剪得掉");
    }

    #[test]
    fn trimming_lets_go_of_the_tile_and_keeps_empty_panes() {
        let _serial = serial();
        let mut t = tree();
        live(1);
        assert_eq!(t.land(&path(&["log"]), tok(1)), Ok(()));
        assert_eq!(t.trim(&path(&["log"])), Ok(()));
        assert!(freed(1));
        assert_eq!(look(&mut t, &path(&["log"])), Err(Fail::Unknown));

        assert_eq!(t.part(&path(&["dev"])), Ok(()));
        assert_eq!(t.trim(&path(&["dev"])), Ok(()));
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
                t.land(&path(&[text.as_str()]), tok(i)),
                Ok(()),
                "第 {i} 条该落得上"
            );
            texts.push(text);
        }
        assert_eq!(names(&t, &[]).map(|v| v.len()), Ok(Operator::PANE_CAP));

        // 令牌是真的、树是满的 ⇒ 这才是 Full
        live(Operator::PANE_CAP);
        assert_eq!(
            t.land(&path(&["n17"]), tok(Operator::PANE_CAP)),
            Err(Fail::Full)
        );
        assert_eq!(t.part(&path(&["n18"])), Err(Fail::Full));

        // 路太深：门槛先于走路
        let deep: Vec<Name> = (0..=Operator::PATH_MAX)
            .map(|i| name(&std::format!("d{i}")))
            .collect();
        live(0);
        assert_eq!(t.part(&deep), Err(Fail::Full));
        assert_eq!(t.land(&deep, tok(0)), Err(Fail::Full));
        assert_eq!(t.trim(&deep), Err(Fail::Full));
        assert_eq!(look(&mut t, &deep), Err(Fail::Full));
        assert_eq!(names(&t, &deep), Err(Fail::Full));
    }

    #[test]
    fn the_root_is_not_an_entry() {
        let _serial = serial();
        let mut t = tree();
        live(1);
        assert_eq!(t.land(&[], tok(1)), Err(Fail::Unknown));
        assert_eq!(t.part(&[]), Err(Fail::Unknown));
        assert_eq!(t.trim(&[]), Err(Fail::Unknown));
        assert_eq!(look(&mut t, &[]), Err(Fail::NotATile));
        assert_eq!(names(&t, &[]), Ok(std::vec![]));
    }
}
