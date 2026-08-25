//! ubi·ucall — U-mode → S-mode 调用构建器/错误。

use erra::ResultExt;
use fack::prelude::Error;

use crate::fid::Ucall;

/// 环境调用结果。
pub type UResult<T> = Result<T, erra::Error<UError>>;

/// 环境调用错误。D1 契约：仅负值构成错误，非负为成功值。
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[error("envcall failed: {0}")]
pub struct UError(isize);

impl UError {
    /// 从 a0 的 signed 解释构造错误码。
    pub fn from_raw(raw: isize) -> Self {
        Self(raw)
    }
}

/// 六寄存器参数载体（a0..a5）。
#[derive(Debug, Clone, Copy, Default)]
pub struct UArgs {
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
}

impl From<UArgs> for [usize; 6] {
    fn from(args: UArgs) -> Self {
        [args.a0, args.a1, args.a2, args.a3, args.a4, args.a5]
    }
}

/// 环境调用构建器。
///
/// `new(call)` 即绑定调用号，`args`/`call` 不可换号；调用号与参数捆绑为类型义务。
pub struct UcallBuilder {
    call: Ucall,
    args: UArgs,
}

impl UcallBuilder {
    pub fn new(call: Ucall) -> Self {
        Self {
            call,
            args: UArgs::default(),
        }
    }

    pub fn args(mut self, args: UArgs) -> Self {
        self.args = args;
        self
    }

    /// 触发并判译：warpper 后按 from_raw 分流。a0 负 → Err，非负 → Ok((a0,a1))。
    pub fn call(self) -> UResult<(usize, usize)> {
        let (v0, v1) = unsafe { warpper(self.call, self.args) };
        if (v0 as isize) < 0 {
            Err(UError::from_raw(v0 as isize)).annotate("u-mode environment call")
        } else {
            Ok((v0, v1))
        }
    }
}

/// 唯一碰汇编的原语：a7=调用号、a0..a5=参数 → U 态 ecall → 读回 a0/a1。
///
/// unsafe：直触寄存器约定、不判错；调用方须为 U 态上下文且 call/args 已按 ABI 摆好。
unsafe fn warpper(call: Ucall, args: UArgs) -> (usize, usize) {
    let (v0, v1);
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("a0") args.a0 => v0,
            inlateout("a1") args.a1 => v1,
            inlateout("a7") usize::from(call) => _,
            inlateout("a2") args.a2 => _,
            inlateout("a3") args.a3 => _,
            inlateout("a4") args.a4 => _,
            inlateout("a5") args.a5 => _,
            options(nostack),
        );
    }
    (v0, v1)
}
