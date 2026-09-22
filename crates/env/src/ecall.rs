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

    /// 是否"条件未就绪"（`-3`）——非阻塞原语的可重试信号。
    pub fn is_busy(&self) -> bool {
        self.0 == -3
    }
}

/// # Safety
/// 唯一碰汇编的原语：a7 = 调用号（slot）、a0..a5 = 参数（packed 数组）→ 任务
/// **`ebreak`** → 读回 a0/a1。
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
#[cfg(target_arch = "riscv64")]
#[inline(never)]
pub unsafe fn trap(slot: usize, args: [usize; 6]) -> (usize, usize) {
    let (v0, v1);
    unsafe {
        core::arch::asm!(
            "ebreak",
            inlateout("a0") args[0] => v0,
            inlateout("a1") args[1] => v1,
            inlateout("a7") slot => _,
            inlateout("a2") args[2] => _,
            inlateout("a3") args[3] => _,
            inlateout("a4") args[4] => _,
            inlateout("a5") args[5] => _,
            options(nostack),
        );
    }
    (v0, v1)
}

/// 非 RISC-V 构建（**只可能是宿主侧测试**）：`ebreak` 入口在这里没有对应物。
///
/// 本 crate 除这一个函数外**全部可移植**（`Wire`/`FromPair`/`Permission`/`Name`/各域
/// 枚举的 `slot`/`pack`/`from_wire` 都不碰架构），故门控这一个函数就把 1300 行 ABI
/// 面变成宿主可测的；`call()` 那条路（唯一会走到这里的）在宿主上必然 panic —— 这是
/// 有意的：**没有汇编就没有调用**，不许静默返回假值。
#[cfg(not(target_arch = "riscv64"))]
pub unsafe fn trap(_slot: usize, _args: [usize; 6]) -> (usize, usize) {
    unimplemented!("env::ecall::trap 只在 riscv64 上有实现（宿主侧测试不应触发真实调用）")
}
