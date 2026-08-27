// 时钟源（clock）— 全内核时间源 + 单位换算
//
// 数据：HERTZ = timebase-frequency（OnceLock<u64>，init 注入后只读）；
//      CYCLE = 启动时计数器读数（uptime 基准，AtomicU64）。
//
// 换算纪律：模块边界一律 core::time::Duration；内部热路径 raw u64 ticks；
// 换算用 u128 中间量、饱和防溢出，无浮点。

use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use fack::prelude::Error;
use riscv::register::time;

use crate::lock::OnceLock;
use crate::machine;

const NANOS_PER_SEC: u128 = 1_000_000_000;

/// 单调时刻：time CSR 刻度（u64 计数器）薄包装。
///
/// 时间区间（Duration）在模块边界折算；本类型用于内部比较与「语义时间点」
/// 传递。回绕安全：差值一律 wrapping/saturating 减法。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant(u64);

/// 时钟初始化错误。
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockError {
    /// DTB 未报告 timebase-frequency（或为 0）。
    #[error("no timebase-frequency in device tree")]
    NoTimebase,
    /// 重复初始化。
    #[error("clock already initialized")]
    AlreadyInit,
}

/// init 注入后只读。
static HERTZ: OnceLock<u64> = OnceLock::new();
/// 启动时刻的计数器读数（uptime 基准）。
static CYCLE: AtomicU64 = AtomicU64::new(0);

/// 初始化时钟：注入 hertz，记录启动时刻。须在任何时间 API 调用之前调用。
///
/// # Errors
///
/// 见 [`ClockError`]。
pub fn init() -> Result<(), ClockError> {
    let hertz = machine::info().hertz;
    if hertz == 0 {
        return Err(ClockError::NoTimebase);
    }
    HERTZ
        .set(hertz as u64)
        .map_err(|_| ClockError::AlreadyInit)?;
    CYCLE.store(time::read() as u64, Ordering::Relaxed);
    Ok(())
}

fn hertz() -> u64 {
    HERTZ.get().copied().expect("clock not initialized")
}

/// 当前时刻（time CSR 计数器刻度）。
pub fn now() -> Instant {
    Instant(time::read() as u64)
}

/// 自启动以来的单调时长（uptime）。
pub fn uptime() -> Duration {
    let boot = CYCLE.load(Ordering::Relaxed);
    ticks_to_duration((time::read() as u64).wrapping_sub(boot))
}

impl Instant {
    /// 内部刻度（arm 目标、热路径比较用）。
    pub fn as_ticks(self) -> u64 {
        self.0
    }

    /// 从刻度构造时刻。
    pub(crate) fn from_ticks(t: u64) -> Instant {
        Instant(t)
    }

    /// 自 earlier 以来的时长（回绕安全；earlier 在未来按零处理）。
    #[allow(dead_code)]
    pub fn elapsed_since(&self, earlier: Instant) -> Duration {
        ticks_to_duration(self.0.wrapping_sub(earlier.0))
    }

    /// 自本时刻以来经过的时长（= now − self）。
    #[allow(dead_code)]
    pub fn elapsed(&self) -> Duration {
        now().elapsed_since(*self)
    }

    /// 本时刻 + Duration（换算饱和防溢出；语义为「最晚到期时刻」）。
    pub fn add(&self, d: Duration) -> Instant {
        Instant(self.0.wrapping_add(duration_to_ticks(d)))
    }

    /// 本时刻 − Duration（换算饱和防溢出；非负语义由调用方保证，wrapping 承担）。
    #[allow(dead_code)]
    pub fn sub(&self, d: Duration) -> Instant {
        Instant(self.0.wrapping_sub(duration_to_ticks(d)))
    }

    /// 自 earlier 以来的时长；earlier 在未来 → None。
    #[allow(dead_code)]
    pub fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> {
        (self.0 >= earlier.0).then_some(ticks_to_duration(self.0 - earlier.0))
    }
}

/// ticks → Duration（u128 中间量、饱和到 Duration 可表达范围）。
pub(crate) fn ticks_to_duration(ticks: u64) -> Duration {
    let ns = (ticks as u128).saturating_mul(NANOS_PER_SEC) / hertz() as u128;
    Duration::from_nanos(ns.min(u64::MAX as u128) as u64)
}

/// Duration → ticks（u128 中间量、饱和到 u64::MAX）。
pub(crate) fn duration_to_ticks(d: Duration) -> u64 {
    let ns = d.as_nanos();
    let t = ns.saturating_mul(hertz() as u128) / NANOS_PER_SEC;
    t.min(u64::MAX as u128) as u64
}
