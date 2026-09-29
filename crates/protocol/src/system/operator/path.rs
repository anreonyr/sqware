//! path — **树上的坐标**：一条 `/` 分开的路（每段一枚名字）。
//!
//! 它是"路"这件事在**两侧**的同一个形状：装配者按它落格（`operator/bridge.rs` 的 `land`）、
//! 客人按它译号（`operator/client.rs` 的 `pane` / `tile` / `seek`）、线上那一格就是它自己
//! （`[长度那一字节][路]`，见下面 `impl env::wire::Span for PathBuf` 那两只手）。
//!
//! # 一对：`Path`（借）＋ `PathBuf`（有）
//!
//! 照 `std::path` 那一对定形：**`Path` 是视图**（unsized，包着一条 `str`——名字已定必须 UTF-8，
//! 故串面就是 `str`／`String`）、**`PathBuf` 是拥有面**（堆上一条串，长大长得了）。
//! `PathBuf: Deref<Target = Path>`，故 `Path` 上的每一手在两半上都叫得出来。
//!
//! **照实记（这一对替掉了什么）**：从前**只有一枚 `Path`**，`segs: [Tag; MAX]` 定容 ＋ `Copy`
//! ＋ 能在 `const` 里造——"`Path` 就是 `Path` ＋ `PathBuf` 合一"。名字那一格改成 `String`
//! 之后那一条走不通了：段是堆上的串，`const` 造不出来（[`crate::system`] 那几处 `pub const DIR`
//! 正是那一手）。于是照 std 拆成两半：**装配期是视图**（`pub const DIR: &Path = Path::new(…)`，
//! `Path::new` 是 `const`，std 同形），**运行期是 `PathBuf`**（`try_join` 接着长大）。
//!
//! # 接口照 `std::path::Path` 定形（差异逐条写在这儿）
//!
//! ```text
//!   std                       本格                     差异
//!   Path::new(s)              Path::new(&'static str)  同形、同为 `const`；**只收规范形**
//!                                                      （开头可有 `/`，其余空段一律不许）
//!   PathBuf::from(s)          PathBuf::try_new(s)      长大可能失败（段 ≤ 8、路 ≤ 255 字节）
//!                                                      ⇒ 答 `Option`；`From` 装不下这个失败域
//!   join(x) -> PathBuf        try_join(x) -> Option<…> 同上（std 那边是 `PathBuf` 会分配，我们不失败
//!                                                      的只有"装得下"那半边）
//!   parent() -> Option<&Path> parent() -> Option<&Path> 同形
//!   file_name() -> &OsStr     file_name() -> Option<&str> 同形（视图段 ⇒ 借出来）
//!   iter() -> &OsStr          iter() -> &str           一条路是**一条串**，段就是它的子切片
//!                                                      ⇒ 与 std 的 `components()` 合成一枚
//!   display() / to_str()      impl Display             段必是合法 UTF-8 ⇒ 直接 Display
//!   is_absolute / has_root    ——                       树上的路都从根数起 ⇒ 恒真/恒假的两格不立
//!   extension / file_stem     ——                       段不是文件名
//!   starts_with / strip_prefix ——                      今天一个读者都没有 ⇒ 不立
//! ```
//!
//! **照实记（"路太长"那一格仍在，只是换了界）**：`Path` 从前的界是"≤ 8 段、每段 ≤ 31 字节"
//! ——后者是那一枚定宽格（32 字节含 NUL）带出来的。今天每段不再有单独的界，**界只剩两条**：
//! 段数 ≤ [`Path::MAX`]、整条路 ≤ [`Path::MAX_LEN`] 字节（线上长度那一字节说得出的范围）。
//! 故"路太长"仍然说得出来（`try_join` 答 `None`），只是判据换成了这两条。

use alloc::string::String;
use core::fmt;
use core::ops::Deref;

use env::wire::Span;

/// 树上的坐标（**借的那一半**）：一条 `/` 分开的路，**零段就是根**。
///
/// 规范形：没有开头的 `/`、没有末尾的 `/`、没有空段（`Path::new` 收的那一形）；整条
/// [`Path::MAX_LEN`] 字节以内、段数 [`Path::MAX`] 段以内。
#[repr(transparent)]
pub struct Path(str);

/// 树上的一条路（**有的那一半**）：堆上一条规范形的串。
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PathBuf {
    road: String,
}

impl Path {
    /// 一条路最多几段（原 `frame::ROAD_MAX`）。
    pub const MAX: usize = 8;

    /// 一条路最多几字节（**线上长度那一字节**说得出的范围）。
    pub const MAX_LEN: usize = 255;

    /// **这一格最长占几字节**（长度那一字节 ＋ 满路）——缓冲与线上那一形都按它。
    pub const LEN: usize = 1 + Self::MAX_LEN;

    /// **根**（零段）：整棵树那一层。
    pub const ROOT: &'static Path = Path::new("");

    /// **一条路**：只收**规范形**的常量（开头那个 `/` 可有可无；末尾或中间的空段一律不许）。
    ///
    /// `const`：装配期那几处（[`crate::system::DIR`] 等）要在 `const` 里造出来。**非法 ⇒ 当场
    /// 编不过**（`const` 求值里 panic）；运行期那一路走 [`PathBuf::try_new`] / [`Path::try_join`]。
    pub const fn new(road: &'static str) -> &'static Path {
        let b = road.as_bytes();
        let skip = match b.first() {
            Some(&b'/') => 1,
            _ => 0,
        };
        let (_, rest) = b.split_at(skip);
        if !canonical(rest) {
            panic!("Path::new: 只收规范形（末尾与中间不许有空段）");
        }
        if rest.len() > Path::MAX_LEN {
            panic!("Path::new: 一条路最多 255 字节");
        }
        if slashes(rest) + 1 > Path::MAX {
            panic!("Path::new: 一条路最多 8 段");
        }
        let rest = match core::str::from_utf8(rest) {
            Ok(rest) => rest,
            Err(_) => panic!("Path::new: 不是 UTF-8"),
        };
        Path::from_str(rest)
    }

    /// 从一段已有的字节借出视图（**规范形由调用方保证**——它是本模块私有的那一手）。
    const fn from_str(road: &str) -> &Path {
        // SAFETY: `Path` 是 `str` 的透明包（`repr(transparent)`），两边同一个布局与同一个元数据。
        unsafe { core::mem::transmute::<&str, &Path>(road) }
    }

    /// **拥有化**：视图 → [`PathBuf`]。**不可失败**——视图一定是规范形、一定在两条界内
    /// （三条入口 [`Path::new`] / [`PathBuf::try_new`] / [`Span`](env::wire::Span) 解码都判过），
    /// 故它不必像 [`PathBuf::try_new`] 那样答 `Option`。
    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf {
            road: String::from(self.as_str()),
        }
    }

    /// 那一条路（**规范形**：没有开头的 `/`；根是空串）。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// **接一段（或几段）**：`self` 后面接上 `leaf`，答一条新路。
    ///
    /// 与 [`Path::new`] 同一套切分口径（`leaf` 可以是 `"principal"`，也可以是 `"principal/ask"`）。
    /// **装不下**（超过 [`Path::MAX`] 段 / [`Path::MAX_LEN`] 字节）⇒ `None`。
    ///
    /// **照实记（为什么没有 std 那样的 `join`）**：std 的 `PathBuf::join` 长大不许失败（它会分配）；
    /// 我们这一条路有个**说不出来的上界**——线上那一格（长度那一字节 ＋ [`Path::MAX_LEN`]）。`join`
    /// 一旦不长这样，读者就要问"它什么时候失败"，而 `Option` 正是那一问的答案。
    pub fn try_join(&self, leaf: &str) -> Option<PathBuf> {
        let mut road = String::from(self.as_str());
        let mut n = if road.is_empty() { 0 } else { road.split('/').count() };
        for seg in leaf.split('/') {
            if seg.is_empty() {
                continue;
            }
            n += 1;
            if n > Path::MAX {
                return None;
            }
            if !road.is_empty() {
                road.push('/');
            }
            if road.len() + seg.len() > Path::MAX_LEN {
                return None;
            }
            road.push_str(seg);
        }
        Some(PathBuf { road })
    }

    /// **上一级**（去掉末段）；**根**答 `None`（与 `"/".parent() == None` 同一条）。
    pub fn parent(&self) -> Option<&Path> {
        let road = self.as_str();
        if road.is_empty() {
            return None;
        }
        match road.rfind('/') {
            Some(at) => {
                let (head, _) = road.split_at(at);
                Some(Path::from_str(head))
            }
            None => Some(Path::ROOT),
        }
    }

    /// **末段**（那一格自己的名字）。
    pub fn file_name(&self) -> Option<&str> {
        self.iter().last()
    }

    /// **逐段**（从根那一头起；空段不产出）。
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.as_str().split('/').filter(|seg| !seg.is_empty())
    }

    /// 段数（**根**是 0）。
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// 是不是**根**。
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl PathBuf {
    /// 一条路（**任意写法进，规范形出**：开头的 `/`、重复的 `/`、末尾的 `/` 都归一）。
    ///
    /// 装不下（段数 / 字节数过界）⇒ `None`——与 [`Path::try_join`] 同一条判据、同一处定义。
    pub fn try_new(road: &str) -> Option<PathBuf> {
        let mut out = String::new();
        let mut n = 0usize;
        for seg in road.split('/') {
            if seg.is_empty() {
                continue;
            }
            n += 1;
            if n > Path::MAX {
                return None;
            }
            if !out.is_empty() {
                out.push('/');
            }
            if out.len() + seg.len() > Path::MAX_LEN {
                return None;
            }
            out.push_str(seg);
        }
        Some(PathBuf { road: out })
    }

    /// **根**（零段）。
    pub fn root() -> PathBuf {
        PathBuf {
            road: String::new(),
        }
    }

    /// 借成视图。
    pub fn as_path(&self) -> &Path {
        Path::from_str(&self.road)
    }

    /// 那一条路（规范形）。
    pub fn as_str(&self) -> &str {
        &self.road
    }
}

/// **std 那一对的三条接口面**：`&Path → PathBuf` 的拥有化（[`Path::to_path_buf`]）＋ 它底下那两条
/// （`ToOwned` 要 `Owned: Borrow<Self>`，`From` 是 std 也有的便利形）。三处都是同一件事。
impl core::borrow::Borrow<Path> for PathBuf {
    fn borrow(&self) -> &Path {
        self.as_path()
    }
}

impl alloc::borrow::ToOwned for Path {
    type Owned = PathBuf;

    fn to_owned(&self) -> PathBuf {
        self.to_path_buf()
    }
}

impl From<&Path> for PathBuf {
    fn from(road: &Path) -> PathBuf {
        road.to_path_buf()
    }
}

impl Deref for PathBuf {
    type Target = Path;

    fn deref(&self) -> &Path {
        self.as_path()
    }
}

/// 规范形：没有末尾的 `/`、没有连续的两个 `/`（开头的那个已由 [`Path::new`] 摘掉）。
///
/// **`const`**：`Path::new` 要在常量那一手上当场判——与运行期那一路（[`PathBuf::try_new`] 归一）
/// 的差别只在"失败怎么办"。
const fn canonical(road: &[u8]) -> bool {
    if let Some(&b'/') = road.last() {
        return false;
    }
    let mut i = 0;
    while i + 1 < road.len() {
        let (_, tail) = road.split_at(i);
        if let Some(&b'/') = tail.first() {
            let (_, next) = tail.split_at(1);
            if let Some(&b'/') = next.first() {
                return false;
            }
        }
        i += 1;
    }
    true
}

/// 一条规范形的路里有几个分隔符（**段数 ＝ 它 ＋ 1**；空的规范形不是一条路，故调用方先判空）。
///
/// **`const`**：`Path::new` 要在常量那一手上把"≤ 8 段"也判掉——`Path` 型自己的义务，
/// 三条入口（常量 / `try_new` / [`Span`] 解码）一处口径。
const fn slashes(road: &[u8]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < road.len() {
        let (_, tail) = road.split_at(i);
        if let Some(&b'/') = tail.first() {
            n += 1;
        }
        i += 1;
    }
    n
}

/// **本格是"要游标那一格"**（[`env::wire::Span`]）：路的长短由内容说。
///
/// 线上那一形：`[长度那一字节][路]`——**只写这一条路的那几字节**（从前是 `[段数][段 × 32]`，
/// 空位也要占满）。读回来那一手**归一 + 判两条界**（段数与字节数），故读出来的路是规范形。
impl Span for PathBuf {
    const MAX: Option<usize> = Some(Path::LEN);

    fn store_at(&self, out: &mut [u8], at: usize) -> Option<usize> {
        let road = self.as_str().as_bytes();
        let end = at.checked_add(1 + road.len())?;
        let span = out.get_mut(at..end)?;
        span[0] = road.len() as u8;
        span[1..].copy_from_slice(road);
        Some(end)
    }

    fn fetch_at(bytes: &[u8], at: usize) -> Option<(PathBuf, usize)> {
        let len = *bytes.get(at)? as usize;
        let end = at.checked_add(1 + len)?;
        let road = core::str::from_utf8(bytes.get(at + 1..end)?).ok()?;
        Some((PathBuf::try_new(road)?, end))
    }
}

impl fmt::Display for Path {
    /// `/svc/sys/principal`——**读数用的那一形**；**根**写 `/`。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("/");
        }
        write!(f, "/{}", self.as_str())
    }
}

impl fmt::Display for PathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_path(), f)
    }
}

impl PartialEq for Path {
    fn eq(&self, other: &Path) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Path {}

impl fmt::Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
