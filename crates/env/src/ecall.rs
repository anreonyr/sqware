//! envcall — U-mode → S-mode 环境调用原语 / 错误 / 汇编入口。
//!
//! 本文件只保留**跨域共用**的调用骨架：失败词汇（`Fail`）、错误读法（`EnvError`）、
//! 结果（`EnvResult`）、唯一汇编入口（`trap`）。各域枚举的 `call()/slot()/pack()` 由
//! `derive(Envcall)` 生成（见 `fid.rs`），它们调用本文件的 `trap`。

/// 环境调用结果。
pub type EnvResult<T> = Result<T, erra::Error<EnvError>>;

/// envcall 的失败词汇。**负码即契约**（D1）：判别值就是 a0 被读成负数时那一格的值，
/// 也是内核侧 `ret_err` 写出去的那张表的**唯一真相**（[`EnvError::code`] 的表由此得来，
/// 不再指向内核）。
///
/// 与 [`EnvError`] 的分工：这一枚是**词汇**（哪个码是什么失败），后者是**对线的读法**
/// （裸 `isize` + 符号 + [`EnvError::is_busy`]）——线上那一格只有数字，没有枚举。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// 权限不足 / 句柄不存在 / 类型不符。
    Denied = -1,
    /// 资源已封印 / 弱引用升不起来。
    Dead = -2,
    /// 条件未就绪（用户态 [`EnvError::is_busy`] 消费这一格）。
    Busy = -3,
    /// 资源耗尽。
    OoM = -4,
    /// 字节数非页对齐 / 非法。
    NotAligned = -5,
    /// 镜像不可装载（parse / 装载任一步失败）。
    BadImage = -6,
    /// 这一枚**已被我交出**（接收方手里那一枚还在）：交回即复原，**不是失败**。
    ///
    /// **照实记（名字的来历）**：它曾叫 `Caged`——那是 `CAGE` 形态位的时代（`aa50a95`
    /// 用 `ONLY` 取代了 CAGE）。机制今天叫"移交"（`ONLY` 形态位 + `Pie.heir` 锚 + 用户态
    /// 的「被关住」判据），故名字换成说"已交出"的这一个；**码 −7 一字未动**（ABI 不变）。
    HandedOver = -7,
}

impl Fail {
    /// 那一格里的负码。**判别值即码**，故实现是 `self as isize`——码表只有一处
    /// （旧形状是一张 `match` 表，与本文件的文档表各写一遍）。
    pub const fn code(self) -> isize {
        self as isize
    }

    /// 七枚词汇（次序就是 [`Fail::of_code`] 走的那一趟）。**它与枚举是两处**，见下。
    const ALL: [Self; 7] = [
        Self::Denied,
        Self::Dead,
        Self::Busy,
        Self::OoM,
        Self::NotAligned,
        Self::BadImage,
        Self::HandedOver,
    ];

    /// **读法**：a0 里那一格负码，是七枚里的哪一枚；表外（含 `0` 与正数）⇒ `None`。
    ///
    /// 这是 [`EnvError`] 那一格的**唯一一道读法**（[`Fail::code`] 的逆）：`HolePie::push`
    /// 那一族报回来的 `erra::Error<EnvError>` 就靠它收成词汇（收的那一手在
    /// `protocol::session::slip` 的 `Slip::ship`）。
    ///
    /// **照实记（为什么不是一张 `match` 码表）**：那会把 `-1..=-7` 在那些支里再写一遍——
    /// 正是 [`Fail::code`] 的注里说的那个旧形状（一张 `match` 表 ＋ 文档表各写一遍）。
    /// 这一趟里**一个数都不写**：比的是判别值自己。
    ///
    /// **照实记（`ALL` 与枚举是两处——已知的空隙）**：Rust 没有"枚举的变体表"
    /// （`core::mem::variant_count` 在本仓这条 nightly 上实测 `E0658`，还在 unstable 口上），
    /// 故加一枚词汇要**两处都改**。兜底是本文件紧接着那条编译期断言：它逐枚验"读得回来"
    /// （**写错**一枚编不过），但**盯不了"漏写"**——新加的那一枚若没进 `ALL`，那一条不会知道。
    pub const fn of_code(code: isize) -> Option<Self> {
        let mut at = 0;
        while at < Self::ALL.len() {
            if Self::ALL[at].code() == code {
                return Some(Self::ALL[at]);
            }
            at += 1;
        }
        None
    }
}

/// 负码即 ABI 契约：七枚码**一个都不许动**（编译期锁死——改一个就是改 ABI）。
/// 同 `layout.rs` / `PAIR_LEN` 那类编译期断言的纪律。
const _: () = {
    assert!(Fail::Denied.code() == -1);
    assert!(Fail::Dead.code() == -2);
    assert!(Fail::Busy.code() == -3);
    assert!(Fail::OoM.code() == -4);
    assert!(Fail::NotAligned.code() == -5);
    assert!(Fail::BadImage.code() == -6);
    assert!(Fail::HandedOver.code() == -7);
};

/// **读法**那一侧也锁死（同一条纪律：写错一枚就是"另一种失败"）：七枚逐枚读得回来、
/// 表外答 `None`。
///
/// **为什么是编译期断言、不是宿主靶**：这一格全是常量，而用户裁定过"**常量交给编译器**"
/// （板那几条"面不相撞"的判据就是这么从运行时用例搬过来的）——故它在**编的时候**红，
/// 比在某一台上红早一步，也不给"少跑一台"留缝。码一律从判别值取，故这一块里一个数都不写。
const _: () = {
    assert!(matches!(
        Fail::of_code(Fail::Denied.code()),
        Some(Fail::Denied)
    ));
    assert!(matches!(Fail::of_code(Fail::Dead.code()), Some(Fail::Dead)));
    assert!(matches!(Fail::of_code(Fail::Busy.code()), Some(Fail::Busy)));
    assert!(matches!(Fail::of_code(Fail::OoM.code()), Some(Fail::OoM)));
    assert!(matches!(
        Fail::of_code(Fail::NotAligned.code()),
        Some(Fail::NotAligned)
    ));
    assert!(matches!(
        Fail::of_code(Fail::BadImage.code()),
        Some(Fail::BadImage)
    ));
    assert!(matches!(
        Fail::of_code(Fail::HandedOver.code()),
        Some(Fail::HandedOver)
    ));
    // 表外：**不是"某一枚失败"**（`0` 与正数按 D1 就不是错误；更负的码这一版不认得）。
    assert!(matches!(Fail::of_code(0), None));
    assert!(matches!(Fail::of_code(1), None));
    assert!(matches!(Fail::of_code(-8), None));
    assert!(matches!(Fail::of_code(isize::MIN), None));
    assert!(matches!(Fail::of_code(isize::MAX), None));
};

/// 环境调用错误。D1 契约：仅负值构成错误，非负为成功值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvError(isize);

impl core::fmt::Display for EnvError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "envcall failed: {}", self.0)
    }
}

/// 把裸错误码包装进 erra::Error（derive(Envcall) 的 call() 用）。
pub fn make_err(e: EnvError) -> erra::Error<EnvError> {
    erra::Error::new("envcall", e)
}

impl EnvError {
    /// 从 a0 的 signed 解释构造错误码。
    pub fn from_raw(raw: isize) -> Self {
        Self(raw)
    }

    /// 错误码。D1 负值契约：**仅负值构成错误，非负是成功值**。
    ///
    /// 每一种码是什么失败，**只有一份账**：[`Fail`] 的判别值（`-1..=-7`）。线上的格子里
    /// 只有数字，故这里只交数字。
    pub fn code(&self) -> isize {
        self.0
    }

    /// 是否"条件未就绪"（[`Fail::Busy`]）——非阻塞原语的可重试信号。
    ///
    /// 判据取自 [`Fail::Busy`] 而**不是**字面 `-3`：本文件头注立的规矩是"码表只有一处"
    /// （`Fail` 的判别值），这一格从前是第二处。
    pub fn is_busy(&self) -> bool {
        self.0 == Fail::Busy.code()
    }
}

/// # Safety
/// 唯一碰汇编的原语：a7 = 调用号（slot）、a0..a5 = 参数（packed 数组）→ 任务
/// **`ebreak`** → 读回 a0..a2。
///
/// 为什么不是 `ecall`：`ecall` 的语义随特权级变化——U 态 `ecall`（scause=8）委派
/// 给 S 态，但 **S 态 `ecall`（scause=9）是 SBI 调用，进 M 态固件**（`medeleg`
/// 位 9 由 OpenSBI 清零）。S 态 supervisor 域任务用 `ecall` 发环境调用会静默
/// 变成一次失败的 SBI 调用（返回值当错误码，陷阱不进内核）。`ebreak`
/// （scause=3）在 U 态与 S 态都被委派给 S 态，故**两类任务共用同一入口**。
///
/// unsafe：直触寄存器约定、不判错；调用方须已按 ABI 摆好 slot/packed args。
///
/// **`#[inline(never)]` 是硬不变量**：该 asm 块一旦被内联进调用方，调用方读回的
/// 返回值会错（实测：同样的 `Collect` 调用，内联时 a0 恒 0，独立函数时正确）。
/// 与仓库对裸 asm 的一贯纪律同源：**内联会改写我读回的寄存器**。
///
/// **为什么读回三格**：绝大多数调用只读 `a0`/`a1`（`a0` 兼作"成 / 不成"那一格），
/// 而**宽返回那一格**（`#[ret3(T)]`，今天只有 [`PieCall::Collect`](crate::PieCall::Collect)）
/// 要多交两件事实出来——`a2` 本来就是这一次调用的 `inlateout`（内核本来就可以写它），
/// 故这里只是**把值绑出来**：不多一次读、也不改任何一格的约定。宽的那一格自己在
/// [`FromTriple`](crate::wire::FromTriple) 里说清 `a0..a2` 各是什么。
#[cfg(target_arch = "riscv64")]
#[inline(never)]
pub unsafe fn trap(slot: usize, args: [usize; 6]) -> (usize, usize, usize) {
    let (v0, v1, v2);
    unsafe {
        core::arch::asm!(
            "ebreak",
            inlateout("a0") args[0] => v0,
            inlateout("a1") args[1] => v1,
            inlateout("a7") slot => _,
            inlateout("a2") args[2] => v2,
            inlateout("a3") args[3] => _,
            inlateout("a4") args[4] => _,
            inlateout("a5") args[5] => _,
            options(nostack),
        );
    }
    (v0, v1, v2)
}

/// 非 RISC-V 构建（**只可能是宿主侧测试**）：`ebreak` 入口在这里没有对应物。
///
/// 本 crate 除这一个函数外**全部可移植**（`Wire`/`FromPair`/`Permission`/`Name`/各域
/// 枚举的 `slot`/`pack`/`from_wire` 都不碰架构），故门控这一个函数就把 1300 行 ABI
/// 面变成宿主可测的；`call()` 那条路（唯一会走到这里的）在宿主上必然 panic —— 这是
/// 有意的：**没有汇编就没有调用**，不许静默返回假值。
#[cfg(not(target_arch = "riscv64"))]
pub unsafe fn trap(_slot: usize, _args: [usize; 6]) -> (usize, usize, usize) {
    unimplemented!("env::ecall::trap 只在 riscv64 上有实现（宿主侧测试不应触发真实调用）")
}
