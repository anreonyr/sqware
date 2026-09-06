//! ubi·ucall — U-mode → S-mode 调用原语 / 错误 / 汇编入口。
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

    /// 错误码（D1 负值契约：-1 = Dead、-2 = Busy 等；mail 端口语义）。
    pub fn code(&self) -> isize {
        self.0
    }
}

/// 唯一碰汇编的原语：a7 = 调用号（slot）、a0..a5 = 参数（packed 数组）→ U 态
/// ecall → 读回 a0/a1。
///
/// unsafe：直触寄存器约定、不判错；调用方须已按 ABI 摆好 slot/packed args。
pub unsafe fn warpper(slot: usize, args: [usize; 6]) -> (usize, usize) {
    let (v0, v1);
    unsafe {
        core::arch::asm!(
            "ecall",
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
