//! board 的核心 —— **公示板、牌子、失败域，与那四个动作**。
//!
//! 本文件**不碰内核**：判据只有一条可机械检查的纪律——
//!
//! > `core.rs` 里不出现 `runtime::`。
//!
//! "谁授的这枚入口""这位还在吗""放下它"全是**注入的事实与动作**（[`Probe`] /
//! [`Alive`] / [`Free`]），故一块板的规矩喂三个假闭包就能推理，换载体不必重写。

use env::{Name, PieToken, TaskId};

// ── 结构 ────────────────────────────────────────────────────

/// 板上的一枚牌子：**叫什么、在哪、谁挂的**。
///
/// 入口是 [`Option`] 而不是哨兵值：**"没有实例"是一个状态**（从未登记 / 已注销 / 已死
/// 三者同一个状态），故 `Unknown` 只有一种语义。
///
/// `owner` 是挂牌人：它在**有实例**时才有意义，而实例一死 `sweep_at` 就把牌子扫空
/// （连 `owner` 一起）——**故它不可能过期**，而扫牌那两条判据里有一条问的正是"这位还在吗"
/// （[`Alive`]），故读的人不必自己再判一遍。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sign {
    name: Name,
    entry: Option<PieToken>,
    owner: Option<TaskId>,
}

impl Sign {
    /// 空牌子：板子的初值。
    pub const VACANT: Sign = Sign {
        name: Name::EMPTY,
        entry: None,
        owner: None,
    };

    /// 这一枚叫什么。
    pub fn name(&self) -> Name {
        self.name
    }

    /// 挂在哪（`None` = 没人挂 / 挂的人已经不在）。
    pub fn entry(&self) -> Option<PieToken> {
        self.entry
    }

    /// 谁挂的（`None` = 同上）。
    pub fn owner(&self) -> Option<TaskId> {
        self.owner
    }

    /// 牌子立着——有实例，故这一枚此刻答得出"在哪"。
    pub fn standing(&self) -> bool {
        self.entry.is_some()
    }

    /// 板上有没有这一枚的名字（诊断：牌子存在，哪怕此刻是空的）。
    pub fn named(&self) -> bool {
        !self.name.is_empty()
    }

    fn lift(&mut self) {
        self.entry = None;
        self.owner = None;
    }
}

/// 挂一枚牌子上板时，坏在哪一步。
///
/// 四个变体各对应**一个不同的下一步**：换个名字 / 摘掉旧牌 / 找持板者要入口 / 扩容。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 板上没这一枚（也没空位放它）。
    Unknown,
    /// 这一枚已经有人挂着了（摘牌或换名，别抢）。
    Taken,
    /// 这枚入口不是你亲手交给持板者的。
    Denied,
    /// 板挂了（条数是策略，容器有界——与 `Service` 表、`Quay` 同款）。
    Full,
}

/// **活性**：这枚入口还在我的表里吗？谁授的？
///
/// `None` = 不在了（令牌越界，或它已被 [`Free`] 放下）——两种情形对牌子是同一件事：
/// **实例没了**。返回的 [`TaskId`] 是授与人（原始自持编码为 `TaskId(0)`）。
pub type Probe = fn(PieToken) -> Option<TaskId>;

/// **这位还在吗**：`who` 这位（人）还在不在。
///
/// 与 [`Probe`] 分工：那一格答的是"这枚入口还在我表里吗"——那是**板自己持有的那一份
/// 副本**在不在；副本正是板持有的，客人死了它照样活着 ⇒ "人还在不在"不能拿副本当问句。
///
/// 答不出（越界 / 没有这一位 / 调用失败）由注入方定成"还在"（**保守方向**：漏剔一格无害，
/// 误剔会把活着的客人静音）。本文件只问这一句。
pub type Alive = fn(TaskId) -> bool;

/// **放下**：把自己那一份入口自释。牌子被换掉或扫空时用它，否则那枚门闩漏在板上。
pub type Free = fn(PieToken) -> Result<(), ()>;

// ── 板 ──────────────────────────────────────────────────────

/// 一块公示板：一叠牌子，每枚写着「叫什么 | 在哪」。
///
/// 三个动作全落在**一枚牌子**上：
///
/// ```text
///   Register    挂上 / 改写自己的牌子        （行侧：我挂 / 我换）
///   Unregister  摘下自己的牌子，牌子留着
///   Query       照牌子上的字说出"在哪"        （问侧：我问 / 我答）
/// ```
///
/// **不存预约表**：谁能用哪个名字是装配期的事（谁拿到那枚入口），不是板子上的判定。
/// 故板只管一条判据：`probe(entry) == Some(who)`——**那枚入口是你亲手交给我的**。
/// 代价照实说：拿到持板者入口的任何服务都能挂**任意未被占用的名字**，"已占用"是唯一
/// 的护栏。
pub struct Board {
    signs: [Sign; Board::CAP],
    probe: Probe,
    alive: Alive,
    free: Free,
}

impl Board {
    /// 板上有多少枚牌子。条数是策略，容器要有界。
    pub const CAP: usize = 16;

    /// 立一块板：三个注入的机制事实（探入口 / 问人 / 放下）跟着板走——它们对每一枚牌子同值，
    /// 故不必逐个作参数传。
    pub const fn new(probe: Probe, alive: Alive, free: Free) -> Board {
        Board {
            signs: [Sign::VACANT; Board::CAP],
            probe,
            alive,
            free,
        }
    }

    /// 挂上（或改写）自己的一枚牌子。返那枚入口。
    ///
    /// 四条判据，一条不多：
    ///
    /// - 那枚入口必须**是我亲手交出去的**（[`Fail::Denied`]）；
    /// - 已有牌子 ⇒ 名字**必须**与它一字不差（板不是改名处，改名就是换一枚牌：
    ///   先 [`Board::unregister`] 再登）；
    /// - 已有**别人**挂着的实例 ⇒ [`Fail::Taken`]；**自己**挂着的 ⇒ 覆盖（重登 / 换绑）；
    /// - 都没占 ⇒ 找一枚空牌子立上，板满则 [`Fail::Full`]。
    pub fn register(&mut self, name: Name, entry: PieToken, who: TaskId) -> Result<PieToken, Fail> {
        if (self.probe)(entry) != Some(who) {
            return Err(Fail::Denied);
        }
        match self.find(name) {
            Some(at) => {
                self.sweep_at(at);
                match self.signs[at].owner {
                    Some(owner) if owner != who => return Err(Fail::Taken),
                    _ => {}
                }
                // 换绑就是**先摘后挂**：上一枚入口是资源实体的一份引用，不放下就漏水。
                // 故这里与 `unregister` 走同一只手下牌。
                self.free_off(at);
                self.signs[at].entry = Some(entry);
                self.signs[at].owner = Some(who);
                Ok(entry)
            }
            None => match self.find_vacant() {
                Some(at) => {
                    self.signs[at] = Sign {
                        name,
                        entry: Some(entry),
                        owner: Some(who),
                    };
                    Ok(entry)
                }
                None => Err(Fail::Full),
            },
        }
    }

    /// 摘下自己的一枚牌子（**牌子留着**：名字的位置不还给别人）。
    ///
    /// 摘的时候把板上那一份入口放下——它是资源实体的一份引用，不放下就漏水。
    ///
    /// **空牌子 = `Unknown`**：没有实例的牌子（从未登记 / 已注销 / 已死）答不出主人是谁，
    /// 故"这名字没人挂着"与"你挂的不是它"是两件事，`Denied` 只留给后者。
    pub fn unregister(&mut self, name: Name, who: TaskId) -> Result<(), Fail> {
        let at = self.find(name).ok_or(Fail::Unknown)?;
        self.sweep_at(at);
        if self.signs[at].entry.is_none() {
            return Err(Fail::Unknown);
        }
        if self.signs[at].owner != Some(who) {
            return Err(Fail::Denied);
        }
        self.free_off(at);
        Ok(())
    }

    /// 摘掉这位挂在板上的**全部**牌子，返摘了几枚（没有它的牌子 ⇒ `0`，**不算错**）。
    ///
    /// 与 [`Board::unregister`] 的分工：那一枚是**逐名**对偶的右半（一次一个名字，叫不出
    /// 名字就摘不下来）；这一句是**整位客人退场**——它挂过的名字一次全放下，故调用方不必
    /// 先知道它挂过哪些名字。两处走同一只手下牌（[`Board::free_off`]），故"牌子留着、
    /// 名字不流转"这条规矩一字不差。
    pub fn free_of(&mut self, who: TaskId) -> usize {
        let mut freed = 0;
        for at in 0..self.signs.len() {
            // `owner` 与 `entry` 同生同灭（`register` 一起写、`lift` 一起清）⇒ 认主人一格
            // 就够了：牌子还立着才数得进这一笔。
            if self.signs[at].owner == Some(who) {
                self.free_off(at);
                freed += 1;
            }
        }
        freed
    }

    /// 照牌子上的字说出"在哪"。答不出只有两种可能，**且都是同一件事**：
    /// 板上没这一枚，或挂着的那一枚已经不在了。
    pub fn lookup(&mut self, name: Name) -> Result<PieToken, Fail> {
        self.lookup_after(name, |_| ())
    }

    /// 同 [`Board::lookup`]，但拿到入口后先交给 `f`（适配层在这里把入口授给调用方，
    /// 免得"先查再授"中间再多一次查找）。
    pub fn lookup_after(
        &mut self,
        name: Name,
        mut f: impl FnMut(PieToken),
    ) -> Result<PieToken, Fail> {
        let at = self.find(name).ok_or(Fail::Unknown)?;
        self.sweep_at(at);
        match self.signs[at].entry {
            Some(entry) => {
                f(entry);
                Ok(entry)
            }
            None => Err(Fail::Unknown),
        }
    }

    /// 板上此刻立着的牌子（诊断 / 将来的枚举：**只报有实例的**）。
    pub fn rows(&self) -> impl Iterator<Item = Sign> + '_ {
        self.signs.iter().copied().filter(Sign::standing)
    }

    /// 名字 → 牌子号。
    pub fn find(&self, name: Name) -> Option<usize> {
        self.signs
            .iter()
            .position(|sign| sign.named() && sign.name == name)
    }

    // ── 惰性剔除：板上不留死实例 ──────────────────────────────

    /// 扫一枚牌子：**挂牌那位不在**（[`Alive`]）**或**入口不在我表里（[`Probe`]）即
    /// **当场把牌子扫空**——不留死实例，也不留一个"实例已死"的中间状态。
    ///
    /// 读路径也扫（[`Board::lookup`] 会扫），故"已死"永远不会被答出去；代价是读也要
    /// 问一次人，那份代价由注入方定价（内核那侧是一次表查询）。
    fn sweep_at(&mut self, at: usize) {
        let Some(entry) = self.signs[at].entry else {
            return;
        };
        // `owner` 与 `entry` 同生同灭 ⇒ 走到这儿主人一定在，认它一格就够。
        let alive = self.alive;
        let probe = self.probe;
        let who_gone = self.signs[at].owner.is_some_and(|who| !alive(who));
        if who_gone || probe(entry).is_none() {
            self.free_off(at);
        }
    }

    /// 摘实例、清主人，**牌子留着**。
    fn free_off(&mut self, at: usize) {
        if let Some(entry) = self.signs[at].entry {
            let _ = (self.free)(entry);
        }
        self.signs[at].lift();
    }

    /// 空牌子号：**从未用过**的才算空（摘过牌的位子留着，名字的位置不流转）。
    fn find_vacant(&self) -> Option<usize> {
        self.signs.iter().position(|sign| !sign.named())
    }
}

// ── 宿主测试（不进 riscv 构建）────────────────────────────────
//
// 跑法与实测有效的那条路写在 `crates/protocol/Cargo.toml` 的 `[lib]` 注记里
// （本 crate 依赖 `runtime`，而它含 RISC-V 汇编 ⇒ 临时宿主驱动 `include!` 本文件）。
//
// 三个注入点就是全部外部依赖，故喂三张假表即可把板上规矩推理干净。

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// 假表是**进程级**的（`Probe` 是函数指针，捕不了环境），故测试彼此串行。
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 甲（挂牌那位）与乙（另一个也拿着入口的服务）。
    const A: TaskId = TaskId::new(1);
    const B: TaskId = TaskId::new(2);

    /// 假表：第 i 位 = 令牌 i 的授与人（0 = 这枚不在我表里）。
    ///
    /// 比板**多一位**：撑满板要 `CAP` 枚活令牌（0..CAP），第 `CAP` 位是没有出处的
    /// 那一枚——"板满了"必须是 `Full`，不能先被活性判成 `Denied`（判据的次序是契约）。
    static TABLE: [AtomicUsize; Board::CAP + 1] = [const { AtomicUsize::new(0) }; Board::CAP + 1];
    static FREED: AtomicUsize = AtomicUsize::new(0);

    /// 「不在的人」集合：第 `who` 位 = 这位已不在（[`Alive`] 的假注入）。
    ///
    /// 与 [`TABLE`] / [`FREED`] 同一套写法（`Alive` 也是函数指针、捕不了环境 ⇒ 进程级静态）。
    /// 本模块的用例赌的是 `Probe` 那条判据，故这本集合留空时 `fake_alive` 一律答"还在"。
    static DEAD: AtomicUsize = AtomicUsize::new(0);

    /// 造一枚号给核心用：**唯一的门是"收号"**（`PieToken::from_bytes`），
    /// 本模块自己不造号（`env::wire::handle`）。
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

    /// 判活：这位还在吗——**答"不在"当且仅当它在 [`DEAD`] 里**；越界同样答"还在"
    /// （与 [`Alive`] 契约同一条：答不出按"还在"办）。
    fn fake_alive(who: TaskId) -> bool {
        let at = who.get();
        at >= usize::BITS as usize || DEAD.load(Ordering::Relaxed) & (1usize << at) == 0
    }

    /// 记下"这一枚被放下了"。假表有 `CAP` 位，越界的令牌不记。
    fn fake_free(entry: PieToken) -> Result<(), ()> {
        if entry.get() < TABLE.len() {
            FREED.fetch_or(1usize << entry.get(), Ordering::Relaxed);
        }
        Ok(())
    }

    fn board() -> Board {
        for slot in &TABLE {
            slot.store(0, Ordering::Relaxed);
        }
        FREED.store(0, Ordering::Relaxed);
        DEAD.store(0, Ordering::Relaxed);
        Board::new(fake_probe, fake_alive, fake_free)
    }

    /// 令牌 `entry` 此刻在我表里，且是 `who` 授的。
    fn mine(entry: usize, who: TaskId) {
        if let Some(slot) = TABLE.get(entry) {
            slot.store(who.get(), Ordering::Relaxed);
        }
    }

    /// 那枚不在了（放下了 / 令牌越界）。
    fn gone(entry: usize) {
        if let Some(slot) = TABLE.get(entry) {
            slot.store(0, Ordering::Relaxed);
        }
    }

    /// 这位不在了（入 [`DEAD`]）——[`gone`] 在判活那一侧的同一件事。
    ///
    /// 本轮一条用例都不叫它（既有用例赌的是 `Probe` 那条判据，上级明令不加测试）⇒ 先挂着
    /// `allow`，等有用例查"挂牌那位不在"那一格时摘掉。
    #[allow(dead_code)]
    fn dead(who: TaskId) {
        DEAD.fetch_or(1usize << who.get(), Ordering::Relaxed);
    }

    fn freed(entry: usize) -> bool {
        FREED.load(Ordering::Relaxed) & (1usize << entry) != 0
    }

    fn name(text: &str) -> Name {
        Name::new(text).unwrap()
    }

    fn stand(b: &mut Board, text: &str, entry: usize, who: TaskId) -> Result<PieToken, Fail> {
        mine(entry, who);
        b.register(name(text), tok(entry), who)
    }

    #[test]
    fn register_requires_the_entry_to_be_mine() {
        let _serial = serial();
        let mut b = board();
        gone(1);
        assert_eq!(b.register(name("console"), tok(1), A), Err(Fail::Denied));
        mine(2, B);
        assert_eq!(b.register(name("console"), tok(2), A), Err(Fail::Denied));
        assert_eq!(stand(&mut b, "console", 1, A), Ok(tok(1)));
        assert_eq!(b.lookup(name("console")), Ok(tok(1)));
    }

    #[test]
    fn a_name_standing_for_someone_else_is_taken() {
        let _serial = serial();
        let mut b = board();
        assert_eq!(stand(&mut b, "console", 1, A), Ok(tok(1)));
        mine(2, B);
        assert_eq!(b.register(name("console"), tok(2), B), Err(Fail::Taken));
        assert_eq!(b.unregister(name("console"), B), Err(Fail::Denied));
        assert!(!freed(1));
        assert_eq!(b.unregister(name("console"), A), Ok(()));
        assert!(freed(1));
        assert_eq!(b.lookup(name("console")), Err(Fail::Unknown));
        assert_eq!(b.unregister(name("console"), A), Err(Fail::Unknown));
    }
    #[test]
    fn repeated_register_overwrites_and_frees_the_old_entry() {
        let _serial = serial();
        let mut b = board();
        stand(&mut b, "console", 1, A);
        stand(&mut b, "console", 2, A);
        assert_eq!(b.lookup(name("console")), Ok(tok(2)));
        assert!(freed(1) && !freed(2));
        assert_eq!(b.find(name("console")), Some(0));
        assert_eq!(b.rows().count(), 1);
    }

    #[test]
    fn a_dead_entry_is_swept_on_the_read_path() {
        let _serial = serial();
        let mut b = board();
        stand(&mut b, "console", 1, A);
        gone(1);
        assert_eq!(b.lookup(name("console")), Err(Fail::Unknown));
        assert!(freed(1));
        assert_eq!(b.rows().count(), 0);
        assert_eq!(b.find(name("console")), Some(0));
        assert_eq!(stand(&mut b, "console", 3, A), Ok(tok(3)));
        assert_eq!(b.lookup(name("console")), Ok(tok(3)));
    }

    #[test]
    fn the_board_has_a_bottom() {
        let _serial = serial();
        let mut b = board();
        let mut texts = std::vec::Vec::new();
        for i in 0..Board::CAP {
            texts.push(std::format!("n{i}"));
            assert_eq!(
                stand(&mut b, &texts[i], i, A),
                Ok(tok(i)),
                "第 {i} 枚该挂得上"
            );
        }
        assert_eq!(b.rows().count(), Board::CAP);
        assert_eq!(
            b.register(name("n17"), tok(Board::CAP), A),
            Err(Fail::Denied),
            "没有出处的令牌先被活性挡住——判据的次序是契约"
        );
        assert_eq!(b.find(name("n17")), None);
        assert_eq!(
            b.register(name("n18"), tok(0), A),
            Err(Fail::Full),
            "令牌是真的、板是满的 ⇒ 这才是 Full"
        );
        assert_eq!(b.find(name("n18")), None);
        assert_eq!(b.unregister(name("n0"), A), Ok(()));
        assert_eq!(
            b.register(name("n18"), tok(0), A),
            Err(Fail::Full),
            "摘过牌的位子不还给新名字——空位只给**从未用过**的牌子"
        );
        assert_eq!(b.find(name("n0")), Some(0), "牌子留着，名字不流转");
        assert_eq!(b.find(name("n18")), None);
    }

    #[test]
    fn unregister_keeps_the_sign_so_the_name_does_not_float_away() {
        let _serial = serial();
        let mut b = board();
        stand(&mut b, "console", 1, A);
        stand(&mut b, "irq", 2, A);
        assert_eq!(b.unregister(name("console"), A), Ok(()));
        assert_eq!(b.rows().count(), 1);
        assert_eq!(b.find(name("console")), Some(0));
        assert_eq!(b.find(name("irq")), Some(1));
        assert_eq!(b.lookup(name("console")), Err(Fail::Unknown));
        assert_eq!(b.lookup(name("irq")), Ok(tok(2)));
        assert_eq!(stand(&mut b, "serial", 3, A), Ok(tok(3)));
        assert_eq!(b.find(name("serial")), Some(2));
    }
}
