//! path — **树上的坐标**：一条最多 [`Path::MAX`] 段的路（每段一枚 [`Name`]）。
//!
//! 它是"路"这件事在**两侧**的同一个形状：装配者按它落格（`operator/bridge.rs` 的 `land`）、
//! 客人按它译号（`operator/client.rs` 的 `pane` / `tile` / `seek`）、线上那一格就是它自己
//! （`[段数][段…]`，见 [`Path::store_in`]）。
//!
//! # 照实记（它为什么迟来；"路太长"那一格因此挪了家）
//!
//! 在它之前，"一条路"是调用点手里那一串 `[Name; N]` ＋ 另一头一个 `usize` 段数：长度这件事
//! 于是每一处各判一次（`count > ROAD_MAX ⇒ FULL`、`count.min(ROAD_MAX)`、`filled`、
//! `&road[..road.len() - 1]`），而拼错一条路只有跑起来才知道。收成一枚类型之后：**长度在
//! 类型里**，段数只有一个作者。
//!
//! 随之**退掉一处失败域**（语义变化只此一处）：从前"路太长"是**说得出来的**（段数那一格能
//! 写 9，持树者据此答 `Full`）；`Path` 里最多 [`Path::MAX`] 段，超长**根本表达不出来** ⇒
//! [`Path::fetch`] 答 `None`、门答 `BAD`（"读不懂"）。`Fail::Full` 于是只剩"那一块 `Pane`
//! 满"一个来源（那两条判据删了，见 `programs/src/system/operator/{answer,core}`）。
//!
//! # 接口照 `std::path::Path` 定形（差异逐条写在这儿）
//!
//! ```text
//!   std                       本格                     差异
//!   Path::new(s)              Path::new(&str)          同形（一整条，`/` 分段）。常量那一手非法 ⇒ 编不过
//!   join(x) -> PathBuf        join / try_join          定容 ⇒ 长大可能失败（多一枚 try_join）；
//!                                                      也没有"绝对段"那回事 ⇒ 恒是接上，不替换
//!   parent() -> Option<&Path> parent() -> Option<Path>  同形；我们给一份（Copy，无第二枚 PathBuf）
//!   file_name()               file_name()              同形（末段）
//!   iter() / components()     iter()                   没有 `.` / `..` 要归一 ⇒ 两枚合一
//!   display() / to_str()      impl Display            段必是合法 UTF-8 ⇒ 直接 Display
//!   PathBuf::{push,pop}       ——                       join / parent 是同一件事的值版本
//!   is_absolute / has_root    ——                       树上的路都从根数起 ⇒ 恒真/恒假的两格不立
//!   extension / file_stem     ——                       段不是文件名
//!   starts_with / strip_prefix ——                      今天一个读者都没有 ⇒ 不立
//! ```
//!
//! **`Path` 就是 `Path` ＋ `PathBuf` 合一**：定容（[`Path::MAX`] 段）＋ `Copy`，故没有第二枚
//! 类型，也没有"借 / 有"那两枚的分工——`&Path` 照样借得到。

use core::fmt;

use env::wire::{NAME_LEN, Name, fetch_tail, store_tail};

/// 树上的坐标：`n` 段名字（`n == 0` 就是**根**）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Path {
    n: u8,
    segs: [Name; Path::MAX],
}

impl Path {
    /// 一条路最多几段（原 `frame::ROAD_MAX`）。**`Path` 的全部容量**：超长造不出来。
    pub const MAX: usize = 8;

    /// **这一格最长占几字节**（段数那一格 ＋ 满路的名字）——缓冲与线上那一形都按它。
    ///
    /// **它为什么不叫 `WIDTH`**（照实记）：[`Field`] 那一族是**定长**的（"帧的偏移全部由它
    /// 求和得出"），而一条路按段数**变长**（只写 `n` 段）。故本格自己给 [`Path::store_in`] /
    /// [`Path::fetch`] 两只手（与 `#[derive(env::Frame)]` 那两只同形），`LEN` 是**最长那一形**
    /// （缓冲那一边按它开）。
    pub const LEN: usize = 1 + NAME_LEN * Path::MAX;

    /// **根**（零段）：整棵树那一层。
    pub const ROOT: Path = Path {
        n: 0,
        segs: [Name::EMPTY; Path::MAX],
    };

    /// **一条路**：按 `/` 分段的一张坐标（`"/svc/sys"` 与 `"svc/sys"` 是同一件事）。
    ///
    /// 读法与 `std::path::Path` 同一条口径：开头的 `/` 可有可无、连续 `/` 的空段跳过、
    /// `""` / `"/"` / `"//"` 都造出**根**。
    ///
    /// **常量那一手**：某一段不合法（≥32 字节 / 含 NUL）、或者段数超过 [`Path::MAX`] ⇒
    /// **当场编不过**（`const` 求值里 panic）。运行期那一路走 [`Path::try_join`]。
    pub const fn new(road: &str) -> Path {
        match walk(Path::ROOT, road) {
            Some(path) => path,
            None => panic!("Path::new: 每段非空且 <32 字节、不含 NUL，最多 8 段"),
        }
    }

    /// **接一段（或几段）**：`self` 后面接上 `leaf`，答一条新路。
    ///
    /// 与 [`Path::new`] 同一套切分口径（`leaf` 可以是 `"principal"`，也可以是 `"principal/ask"`）。
    ///
    /// **照实记（与 `std::path::Path::join` 的两处差）**：① std 那边长大不许失败（`PathBuf`
    /// 会分配），我们定容 ⇒ 另有一枚 [`Path::try_join`]；② std 那边 `join` 一条**绝对**路会
    /// 整条替换，我们这里没有"绝对段"那回事（树上的路都从根数起）⇒ **恒是接上**。
    pub const fn join(&self, leaf: &str) -> Path {
        match walk(*self, leaf) {
            Some(path) => path,
            None => panic!("Path::join: 每段非空且 <32 字节、不含 NUL，最多 8 段"),
        }
    }

    /// [`Path::join`] 的运行期那一半：段不合法 / 装不下 ⇒ `None`（不 panic）。
    ///
    /// **它是"名字来自报文"那几处的落点**：那一侧本就要报一行读数（"哪一格没落上"），
    /// 故这一手答 `Option`，由调用点说那句话。
    pub fn try_join(&self, leaf: &str) -> Option<Path> {
        walk(*self, leaf)
    }

    /// **上一级**（去掉末段）；**根**答 `None`（与 `"/".parent() == None` 同一条）。
    pub fn parent(&self) -> Option<Path> {
        (self.n > 0).then(|| Path {
            n: self.n - 1,
            segs: self.segs,
        })
    }

    /// **末段**（那一格自己的名字）。
    pub fn file_name(&self) -> Option<&Name> {
        self.used().last()
    }

    /// **逐段**（从根那一头起）。
    pub fn iter(&self) -> core::slice::Iter<'_, Name> {
        self.used().iter()
    }

    /// 段数（**根**是 0）。
    pub fn len(&self) -> usize {
        self.n as usize
    }

    /// 是不是**根**。
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// 那几段（线格式那两只手与 [`Path::iter`] 都用它；对外只有后者）。
    fn used(&self) -> &[Name] {
        &self.segs[..self.n as usize]
    }

    /// **编**：段数那一格 ＋ 那几段（`1 + n × 32` 字节）——**只写 `n` 段**，空位不上线。
    pub fn store_in(&self, out: &mut [u8]) -> Option<usize> {
        *out.first_mut()? = self.n;
        store_tail(out, 1, self.used())
    }

    /// **从一段字节的头上读一条路**，返"这一条路 ＋ 读完之后的游标"。
    ///
    /// 给"路之后还有别的格"那几形用（`Tip::Plate`：路 ＋ 末段那一枚 ＋ 规矩一格）——它只管读
    /// 一段，长度对不对**由调用方按它自己那张表判**（与 [`fetch_tail`] 交回游标同一条口径）。
    ///
    /// 段数越界（> [`Path::MAX`]）或那几段读不齐 ⇒ `None`（这一帧读不懂）。
    pub fn take(bytes: &[u8]) -> Option<(Path, usize)> {
        let n = *bytes.first()? as usize;
        if n > Path::MAX {
            return None;
        }
        let end = 1 + n * NAME_LEN;
        if bytes.len() < end {
            return None;
        }
        let mut segs = [Name::EMPTY; Path::MAX];
        fetch_tail(bytes, 1, &mut segs[..n])?;
        Some((Path { n: n as u8, segs }, end))
    }

    /// **解**：这一整段字节就是一条路（`Req::Road` 那一形）——多一字节少一字节都读不懂。
    pub fn fetch(bytes: &[u8]) -> Option<Path> {
        let (path, end) = Path::take(bytes)?;
        (bytes.len() == end).then_some(path)
    }
}

impl fmt::Display for Path {
    /// `/svc/sys/principal`——**读数用的那一形**（与 [`Path::new`] 收的那一形互逆：
    /// `Path::new(&path.to_string()) == path`）；**根**写 `/`。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("/");
        }
        for seg in self.iter() {
            write!(f, "/{seg}")?;
        }
        Ok(())
    }
}

/// 走一遍 `road`（一条 `/` 分开的字符串），把每一段接在 `from` 后面。
///
/// **一段都没有**（`""` / `"/"` / `"//"`）⇒ 原样交回 `from`（空段跳过——与 std 的
/// `components()` 同一条口径）。不合法（某段 ≥32 字节 / 含 NUL）、或者装不下
/// （超过 [`Path::MAX`] 段）⇒ `None`。
///
/// **它是 `const`**：常量那一手（[`Path::new`] / [`Path::join`]）与运行期那一手
/// （[`Path::try_join`]）走的是同一段字节——两样只在"失败怎么办"。
const fn walk(mut from: Path, road: &str) -> Option<Path> {
    let bytes = road.as_bytes();
    let mut i = 0;
    let mut start = 0;
    while i <= bytes.len() {
        if i == bytes.len() || bytes[i] == b'/' {
            // 一段：`[start, i)`。空段（`//` 与末尾那个 `/`）跳过，故这里一定非空。
            //
            // **为什么用 `split_at` 而不是 `&bytes[start..i]`**（照实记）：那一条下标式切片
            // 要走 `Index` 那个 trait，而它在 `const` 里**还不是 stable**（issue #143874，
            // 要 `#![feature(const_index)]`——本仓不开 feature 门）。`split_at` 自 1.71 起
            // 是 `const`，切出来的还是同一段字节；越界那一步到不了（`start ≤ i ≤ len` 由上面
            // 这一趟扫描保证）。
            if i > start {
                let (_, from_start) = bytes.split_at(start);
                let (seg_bytes, _) = from_start.split_at(i - start);
                let seg = match Name::from_slice(seg_bytes) {
                    Ok(seg) => seg,
                    Err(_) => return None,
                };
                if from.n as usize >= Path::MAX {
                    return None;
                }
                from.segs[from.n as usize] = seg;
                from.n += 1;
            }
            start = i + 1;
        }
        i += 1;
    }
    Some(from)
}
