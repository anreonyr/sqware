//! envcall — U-mode → S-mode 环境调用原语 / 错误 / 汇编入口。
//!
//! 本文件只保留**跨域共用**的调用骨架：域失败词汇的共同部分（`FailCode`）、
//! 唯一汇编入口（`trap`）、以及"把词汇装进 `erra::Error`"那一手（`make_fail`）。
//!
//! **码的单一真相在各域词表里**（`fid.rs`，`#[derive(Fail)]` 生成）：域内自 `-1` 起、
//! 判别值即码。从前这里还有一枚**全局**七枚码表（`Fail`）与一层无类型的读法
//! （`EnvError`/`EnvResult`）——"按域分持"那一刀把它们整个退掉了：同一个条件在不同域
//! 不同号，**读法按域**（调用点知道自己在调哪一域），故不需要也不该有全局码表。
//! 各域枚举的 `slot()/pack()/from_wire()` 与**每格一个入口**由 `derive(Envcall)` 生成
//! （见 `fid.rs`），它们都调本文件的 `trap`。

/// **域失败词汇的共同部分**：只有"码"。
///
/// 各域的词汇由 `#[derive(Fail)]`（`mold`）生成——**域内自 `-1` 起**，判别值即码；
/// 同一个条件在不同域不同号，读法按域（调用点知道自己在调哪一域）。这一个 trait 是
/// "把任意域的词汇当失败读"的最小面：内核的 `ret_err` 与上层的 `From` 链只认它。
/// `Display` 由 derive 一并给出（`<域>:<变体名>`），故日志不必看号猜域。
pub trait FailCode: Copy + core::fmt::Debug {
    /// 那一格里的负码。
    fn code(self) -> isize;
}

/// 把域词汇装进 `erra::Error`（derive 生成的每格入口用）。
pub fn make_fail<F: FailCode + core::fmt::Display>(f: F) -> erra::Error<F> {
    erra::Error::new("envcall", f)
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
