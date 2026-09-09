//! envcall·ucall — U-mode → S-mode 调用原语 / 错误 / 汇编入口。
//!
//! 本文件只保留**跨域共用**的调用骨架：错误（`EnvError`）、结果（`EnvResult`）、
//! 唯一汇编入口（`warpper`）。各域枚举的 `call()/slot()/pack()` 由 `derive(Envcall)`
//! 生成（见 `fid.rs`），它们调用本文件的 `warpper`。

/// 环境调用结果。
pub type EnvResult<T> = Result<T, erra::Error<EnvError>>;

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

    /// 错误码。D1 负值契约（单一真相；内核来源 = `work/unit/gate::GateError::code`）：
    ///
    /// | code | 含义 | 内核来源 |
    /// |------|------|----------|
    /// | -1 | Denied（无权 / 无此句柄 / 类型不符） | `GateError::Denied` |
    /// | -2 | Dead（资源已封印） | `GateError::Dead` |
    /// | -3 | Busy（条件未就绪） | `GateError::Busy` |
    /// | -4 | OoM（资源耗尽） | `GateError::OoM` |
    /// | -5 | NotAligned（字节数非页对齐） | `GateError::NotAligned` |
    /// | -6 | BadImage（镜像不可装载） | `UnitError::Load`（parse/装载任一步失败） |
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
/// 与仓库对裸 asm 的一贯纪律同源（见 `docs/ipc.md` §13.10 A.2 的闭包边界锁）。
#[inline(never)]
pub unsafe fn warpper(slot: usize, args: [usize; 6]) -> (usize, usize) {
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
