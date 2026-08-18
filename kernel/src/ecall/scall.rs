//  S-Mode -> M-Mode

use erra::ResultExt;
use fack::prelude::Error;

use super::extension::Extension;

pub type SResult<T> = Result<T, erra::Error<SError>>;

/// SBI 调用参数（对应 a0 ~ a5 寄存器）
#[derive(Debug, Clone, Copy, Default)]
pub struct SArgs {
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
}

impl From<SArgs> for [usize; 6] {
    fn from(args: SArgs) -> Self {
        [args.a0, args.a1, args.a2, args.a3, args.a4, args.a5]
    }
}

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(isize)]
pub enum SError {
    #[error("call Succeed")]
    Success,
    #[error("call failed")]
    Failed,
    #[error("call not supported")]
    NotSupported,
    #[error("invalid parameter(s)")]
    InvalidParam,
    #[error("access denied")]
    Denied,
    #[error("invalid address")]
    InvalidAddress,
    #[error("resource already available")]
    AlreadyAvailable,
    #[error("unknown error: {0}")]
    Unknown(isize),
}

impl SError {
    pub fn from_raw(raw: isize) -> Self {
        match raw {
            0 => SError::Success,
            -1 => SError::Failed,
            -2 => SError::NotSupported,
            -3 => SError::InvalidParam,
            -4 => SError::Denied,
            -5 => SError::InvalidAddress,
            -6 => SError::AlreadyAvailable,
            other => SError::Unknown(other),
        }
    }
}

pub struct ScallBuilder<E: Extension> {
    fid: E::Fid,
    args: SArgs,
}

impl<E: Extension> ScallBuilder<E> {
    pub fn new(fid: E::Fid) -> Self {
        Self {
            fid,
            args: SArgs::default(),
        }
    }
    pub fn args(mut self, args: SArgs) -> Self {
        self.args = args;
        self
    }
    pub fn call(self) -> SResult<usize> {
        unsafe { warpper(E::EID, self.fid.into(), self.args.into()) }
    }
}

unsafe fn warpper(eid: usize, fid: usize, args: [usize; 6]) -> SResult<usize> {
    let [a0, a1, a2, a3, a4, a5] = args;
    let (error, value);
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a0") a0,
            in("a1") a1,
            in("a2") a2,
            in("a3") a3,
            in("a4") a4,
            in("a5") a5,
            in("a6") fid,
            in("a7") eid,
            lateout("a0") error,
            lateout("a1") value,
        );
    }
    let e = SError::from_raw(error);
    match e {
        SError::Success => Ok(value),
        _ => Err(e).annotate("s-mode environment call"),
    }
}
