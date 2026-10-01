//! coalition::frame 的**词汇那一半**：盟号（`CoalitionId`）· 失败词汇（`Fail`）· 窗口（`Window`）·
//! 动作码与状态码 · 记号与那一段路（`BACK`/`DIR`/`NAME`）。

use crate::wire::id::Id;
use env::{Mark };

use crate::common::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CoalitionId(usize);

impl CoalitionId {
    /// 由裸号造一个（线上解码面；没铸过的号从这里进来）。
    pub const fn new(raw: usize) -> CoalitionId {
        CoalitionId(raw)
    }

    /// 裸号。
    pub const fn get(self) -> usize {
        self.0
    }
}

/// 失败域：**三格**，每格一个**不同的下一步**。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 这枚盟不存在（从来没铸过），或这个 TID 没绑过。调用方要改的是：**我手里这个号是假的**
    /// 或**我还没有身份**。
    Unknown,
    /// `try_reserve` 备不下。调用方要改的是：**晚点再来**。
    Full,
    /// 你手里那一枚门牌给不了这一条：**换一枚**（或换一位客人），别重试。
    Denied,
    /// **你不是这一枚盟的盟主** ⇒ 别拿它来代报名（要改的是"换一条路"，不是"再试一次"）。
    NotChief,
}

/// 一窗最多几枚号。条数是策略、容器要有界 ⇒ 窗口有顶，**"还有没有"由 `more` 说**。
pub const WINDOW_CAP: usize = 16;

/// 一窗号：**一趟读的读数**（最多 [`WINDOW_CAP`] 枚，**号序升序**）。
/// 空位是 `None` 而不是 `T::new(0)`：**零号是真格子**（`PrincipalId::ROOT` 就是 0），
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window<T: Id> {
    items: [Option<T>; WINDOW_CAP],
    n: usize,
    more: bool,
}

impl<T: Id> Window<T> {
    /// 空的那一串（`more = false`）。
    pub const fn new() -> Window<T> {
        Window {
            items: [None; WINDOW_CAP],
            n: 0,
            more: false,
        }
    }

    /// 由一串号凑一窗（`more` = 窗外还有）——**解码面**：线上收来的那一窗由这里成形。
    /// 收够 [`WINDOW_CAP`] 枚就停：帧长了是帧的毛病，读的人只认窗前这些（帧长与条数对不对
    /// 由 `protocol` 那一侧的 `frame` 那一层先挡掉）。
    pub fn gather(more: bool, ids: impl Iterator<Item = T>) -> Window<T> {
        let mut out = Window::new();
        for id in ids.take(WINDOW_CAP) {
            out.push(id);
        }
        out.more = more;
        out
    }

    /// 几枚。
    pub fn len(&self) -> usize {
        self.n
    }

    /// 窗外还有没有（这一趟没答完的那些）。
    pub fn more(&self) -> bool {
        self.more
    }

    /// 第 `at` 枚（号序；越界 ⇒ `None`）。
    pub fn get(&self, at: usize) -> Option<T> {
        if at < self.n {
            self.items.get(at).copied().flatten()
        } else {
            None
        }
    }

    /// 号序走一遍。
    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        self.items[..self.n].iter().filter_map(|slot| *slot)
    }

    /// 末一枚——**它就是下一页的游标**（空窗 ⇒ `None`）。
    pub fn last(&self) -> Option<T> {
        self.n.checked_sub(1).and_then(|at| self.get(at))
    }

    /// **取窗那一侧用**：收一枚。收下了 ⇒ `true`；**已经满了** ⇒ `false` 并点亮
    /// [`Window::more`]（"这一趟没答完"）。
    pub fn put(&mut self, id: T) -> bool {
        if self.full() {
            self.more = true;
            return false;
        }
        self.push(id);
        true
    }

    /// 收一枚（再满就丢：取窗那边收了 [`WINDOW_CAP`] 枚就停）。
    fn push(&mut self, id: T) {
        if let Some(slot) = self.items.get_mut(self.n) {
            *slot = Some(id);
            self.n += 1;
        }
    }

    /// 装满了。
    fn full(&self) -> bool {
        self.n == WINDOW_CAP
    }
}

/// 七条线上动作——**与核心那七条原语同名**：线上与模型是同一件事的两层，不该各起一套词。
pub const FOUND: u8 = 1;

pub const ENTER: u8 = 2;

pub const LEAVE: u8 = 3;

pub const AMID: u8 = 4;

pub const BAND: u8 = 5;

pub const BLOC: u8 = 6;

/// **代报名**：把**另一位**放进盟主自己立的那一枚盟。
pub const ADMIT: u8 = 7;

/// 答话那一格：失败域那三格 + "读不懂"。
/// [`BAD`] 在失败表外（同板 / 树 / 身份服务那三家的先例）：它不是"哪个协议说的事"，
/// 是**这一问读不懂**。
pub const UNKNOWN: u8 = 1;

pub const FULL: u8 = 2;

pub const BAD: u8 = 3;

pub const DENIED: u8 = 4;

/// 你不是这一枚盟的盟主：代报名只有**立它那位**做得成。
pub const NOT_CHIEF: u8 = 5;

// 长度、编 / 解、答话那几手**本体在 [`crate::frame`]**——coalition 与 principal 同形（这一族
// 的帧就是照它立的），故只有一份；这里只按本族的名字转出来（`mod.rs` 那一句
// 点名转出照旧，调用点一处都不用改）。

/// 回信孔的记号：客人**每趟**铸一枚、借给 Server（这一趟的答话从它回来）。
/// 与另几面的 `*-back` 同一个形状、不同的记号：同一张表里两面的回信孔若刻同一个记号，
/// 就分不出这一枚是哪一面的。
pub const BACK: Mark = Mark::of("coalition-back");

/// **本族那块窗格在树上的路**：`/svc/sys/coalition`（头两段是四族共用的
/// [`crate::common::svc::DIR`]，末段是本族自己的名字 [`NAME`]）——**一处说全**（同 principal）。
pub const DIR: &Path = Path::new("svc/sys/coalition");

/// 本服务在树上的那一段名字：`/svc/sys/coalition`——**它不是一格**（
/// 两枚门牌是它底下那两格 `/svc/sys/coalition/{ask,set}`，末段名由
/// [`Grant::name`](super::grant::Grant::name) 给）。
pub const NAME: &str = "coalition";
