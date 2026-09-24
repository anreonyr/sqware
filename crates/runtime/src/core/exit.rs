//! exit — **域侧的退场**：`main` 的返回类型（[`Report`] / [`Exit`]）与把它送进内核的那一手
//! （[`finish`]）。
//!
//! # 出口点的形状（照 `std` 那套 `Termination`，改了两处名）
//!
//! `std` 里 `main` 的返回类型是**它自己的事**（`()` / `ExitCode` / `Result` / `!` 都行），
//! 编译器生成的入口去叫 `Termination::report()` 把它折成一个 **`ExitCode`**，再交给
//! `std::process::exit`。这里一模一样，只换名与那个"折出来的数"：
//!
//! | `std` | 这里 |
//! |---|---|
//! | `Termination` | [`Exit`] |
//! | `report(self) -> ExitCode` | [`Exit::report`] → [`Report`]（携 [`env::Reason`]） |
//! | lang item `#[lang_start]` + `rustc_main` | 生成物里那个 `clean_ret` + `programs::entry::entry` |
//! | `impl Termination for ()/{!}/Result<T,E>` | 同形几条，见本文件 |
//!
//! # 照实记（这些为什么住 `runtime`，而不是 `env` 或 `programs`）
//!
//! 它们原先住 `programs/src/entry.rs`——与 `_start` 的汇编、panic 处理同处一个文件。但那
//! 里其实是**两件事**：一半是**退场的词汇**（"`main` 想对内核说的全部" = `Reap { reason,
//! note }` 那两格），另一半是**入口那一手**（汇编、panic、`entry`）。
//!
//! 词汇该住哪，只有一条判据，而且是可 grep 的：**内核读不读它**。
//!   - [`env::Reason`] 与三枚码内核读（`EXIT_FAULT` 由故障隔离路径落账）⇒ 住 `env::exit`；
//!   - [`Report`] / [`Exit`] / [`finish`] 内核一处也不碰，而 [`finish`] 要叫
//!     [`crate::env::room::exit`]（`env` 不能依赖 `runtime`）⇒ **住这里**。
//!
//! 于是 `programs/src/entry.rs` 只剩**入口那一手**，`programs` 的 crate 根照旧转出
//! [`Exit`] / [`Report`]——**调用点一行没改**（各 bin 的 `main` 仍写 `-> Report<'static>`）。

use env::{EXIT_OK, Reason};

use crate::env::room::exit;

/// note 的**线形状**：`(ptr, len)`——`0` 长度即"无话"。
///
/// **为什么不用 `Option<&str>`**：这个词要跨 `programs::entry::entry` 那个
/// `extern "C" fn clean_ret()` 的边界回来，而 `Option<&str>` 是带 niche 的枚举、没有
/// `repr`，编译器（正确地）不肯认它是 FFI-safe。拆成两个标量（与内核 `Reap { note, len }`
/// 同一形）就干净了。
#[derive(Clone, Copy)]
#[repr(C)]
struct RawNote {
    ptr: *const u8,
    len: usize,
}

impl RawNote {
    /// 无话。
    const NONE: Self = Self {
        ptr: core::ptr::null(),
        len: 0,
    };

    const fn of(s: &str) -> Self {
        Self {
            ptr: s.as_ptr(),
            len: s.len(),
        }
    }

    /// 解回那句话（只有 [`Report::parts`] 这一个读点，故 SAFETY 条件收在那里）。
    unsafe fn get<'a>(self) -> Option<&'a str> {
        match self.len {
            0 => None,
            n => Some(unsafe {
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(self.ptr, n))
            }),
        }
    }
}

/// 出口点的返回类型：**原因码 + 可选的一句话**——与 [`exit`] 的入参同形。
///
/// 它就是"`main` 想对内核说的全部"：两格，与 `Reap { reason, note }` 一一对应。
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Report<'a> {
    pub reason: Reason,
    note: RawNote,
    _borrow: core::marker::PhantomData<&'a str>,
}

impl<'a> Report<'a> {
    /// 只报码。
    pub const fn new(reason: Reason) -> Self {
        Self {
            reason,
            note: RawNote::NONE,
            _borrow: core::marker::PhantomData,
        }
    }

    /// 报码 + 一句话（"哪里算不下去"）——那句话借多久都行（只要活到出口）。
    pub const fn note(reason: Reason, note: &'a str) -> Self {
        Self {
            reason,
            note: RawNote::of(note),
            _borrow: core::marker::PhantomData,
        }
    }

    /// 拆成 [`exit`] 那两格（[`finish`] 用的就是它）。
    ///
    /// SAFETY：`note` 那一对指针要么是 [`RawNote::NONE`]，要么指着 [`Report::note`] 收进来的
    /// 那个 `&'a str`——`'_` 绑在 `&self` 上，故那句话至少活到本调用返回。
    pub fn parts(&self) -> (Reason, Option<&'a str>) {
        (self.reason, unsafe { self.note.get() })
    }
}

/// `main` 的返回类型只需要这一个 trait——**每个 bin 的 `main` 是它的一个实现**。
///
/// 四条实现就是 `std` 那四条再加一条：`()` = "没有失败要报"（[`EXIT_OK`]）、`!` = "我自己退"
/// （[`exit`] 那条路走不到，但类型上必须能过）、[`Reason`] = 只报码、`Result<T, E>` = `?`
/// 一路带出来。**`()` 不是"忘了报"**：真要报失败的程序把 `main` 写成 `Result<(), Reason>`，
/// 那里 `()` 当不了退出码，忘了报就是编译错误。
///
/// 折出来的不是裸码而是 [`Report`]：**note 也得有地方住**——探针那族的回执就是"报码 +
/// 带一句话"，而那句话常常是**栈上现拼的**（`format!`），故 [`Report`] 借它、不要求 `'static`。
/// 这正是 `report` 取 `&self` 而不是 `self` 的理由：借出的 note 活不过一个按值消耗的 `self`。
pub trait Exit {
    fn report(&self) -> Report<'_>;
}

impl<'a> Exit for Report<'a> {
    fn report(&self) -> Report<'_> {
        Report {
            reason: self.reason,
            note: self.note,
            _borrow: core::marker::PhantomData,
        }
    }
}

impl Exit for Reason {
    fn report(&self) -> Report<'_> {
        Report::new(*self)
    }
}

impl Exit for () {
    fn report(&self) -> Report<'_> {
        Report::new(EXIT_OK)
    }
}

impl Exit for ! {
    fn report(&self) -> Report<'_> {
        match *self {}
    }
}

impl<T: Exit, E: Exit> Exit for Result<T, E> {
    fn report(&self) -> Report<'_> {
        match self {
            Ok(t) => t.report(),
            Err(e) => e.report(),
        }
    }
}

/// 把一份 [`Report`] 送进内核——**`programs::entry::entry` 走到底就是它**（生成物里没有
/// 泛型，这一步因此与 `main` 的返回类型无关）。
///
/// 这里同时让 `reason` 在 `exit(...)` 返回（不该发生）时留在现场，读的人不至于只看到一句
/// `unreachable`。
pub fn finish(report: Report<'_>) -> ! {
    let reason = report.reason;
    let (_, note) = report.parts();
    exit(reason, note)
}
