//! 显式原始资源适配：孔 token 操作与能力表查询。
//!
//! 借入的 token 未经本模块验证，Hole 不拥有资源。Capability 管理本地创建的能力，Loan
//! 管理指定对端的派生授予；这些入口用于协议、硬件和测具边界。

use env::{
    HoleCondition, HoleResult, Mark, PieInfo, PieKind, PieResult, PieToken, TaskId, VirtAddr, Wait,
};

use crate::{cleanup::cleanup, native::Native};

pub use self::hole::Hole;
pub use self::pies::{Pies, pies};

/// 默认后端下的本任务能力。
pub type Capability = crate::capability::Capability<Native>;
/// 默认后端下授给指定对端的一份派生能力。
pub type Loan<'a> = crate::loan::Loan<'a, Native>;

#[path = "raw/hole.rs"]
mod hole;
#[path = "raw/pies.rs"]
mod pies;

/// Complete cleanup despite temporary contention, yielding between attempts.
pub fn release(token: PieToken) -> PieResult<()> {
    cleanup::<Native>(|| env::pie::release(token, env::ReleaseMode::Revoke))
}

pub fn revoke(peer: TaskId, token: PieToken) -> PieResult<()> {
    cleanup::<Native>(|| env::pie::revoke(peer, token))
}

/// 单调时钟读数（纳秒）——deadline 用（机器无关，不依赖 timebase 频率）。
fn now_ns() -> u64 {
    env::chrono::clock()
}

fn deadline_of(within: Wait) -> u64 {
    match within {
        Wait::Forever => u64::MAX,
        Wait::AtMost(ms) => now_ns().saturating_add((ms as u64).saturating_mul(1_000_000)),
    }
}

/// 下一次进内核还要等多久。**至少 1 毫秒**：`AtMost(0)` 在那一侧不是"再看一眼"而是"挂 0 秒"。
fn remains(within: Wait, deadline: u64) -> Wait {
    if matches!(within, Wait::Forever) {
        return Wait::Forever;
    }
    match deadline.checked_sub(now_ns()) {
        None | Some(0) => Wait::AtMost(1),
        Some(d) => Wait::AtMost(((d / 1_000_000) as usize).max(1)),
    }
}

/// 只手递上去（`len ≥ 1`）。**内核那一格的一次尝试**，成不成看孔上有没有手。
fn put(token: PieToken, msg: &[u8]) -> HoleResult<()> {
    env::hole::push(token, VirtAddr::new(msg.as_ptr() as usize), msg.len())
}

/// A capability table entry, including kind, permissions and liveness.
pub type Pie = PieInfo;

/// 本端这张权限表里现在有几枚门闩（[`pies`] 数一遍）。
///
/// **给人看的读数，不是给判据用的机制**：它自己不改任何东西。用途只有一个——把"该放下的
/// 放下了没有"变成**可量**的一格（少放一枚，这一格当场大 1，见
/// `programs/src/driver/router/adapt/desk.rs` 的 `drop_lane` 与 `programs/src/harness/guest/lodger/main.rs`）。
pub fn table_size() -> usize {
    pies().count()
}

/// Query a held reference, including a sealed resource's owner and mark.
pub fn inspect(token: PieToken) -> PieResult<PieInfo> {
    let mut words = [0; PieInfo::WORDS];
    env::pie::inspect(token, VirtAddr::new(words.as_mut_ptr() as usize))?;
    Ok(PieInfo::from_words(words).expect("invalid PieInfo"))
}

/// Hole provenance convenience wrapper over Inspect.
pub fn reserve(token: PieToken) -> PieResult<(TaskId, TaskId, Mark)> {
    let info = inspect(token)?;
    if !info.alive {
        return Err(env::make_fail(env::PieFail::Dead));
    }
    if info.kind != PieKind::Hole {
        return Err(env::make_fail(env::PieFail::Denied));
    }
    Ok((info.vestor, info.owner, info.mark))
}

pub fn alive(token: PieToken) -> bool {
    inspect(token).is_ok_and(|info| info.alive)
}

/// 等资源就绪；无关唤醒后继续使用剩余预算。
pub(crate) fn wait(token: PieToken, dir: HoleCondition, within: Wait) -> HoleResult<bool> {
    wait_for(within, |step| env::hole::wait(token, dir, step))
}

pub(crate) fn wait_for<E>(
    within: Wait,
    mut ready: impl FnMut(Wait) -> Result<bool, E>,
) -> Result<bool, E> {
    let deadline = if within == Wait::POLL {
        0
    } else {
        deadline_of(within)
    };
    if ready(within)? {
        return Ok(true);
    }
    if matches!(within, Wait::AtMost(0)) {
        return Ok(false);
    }
    loop {
        if now_ns() >= deadline {
            return Ok(false);
        }
        let step = remains(within, deadline);
        if ready(step)? {
            return Ok(true);
        }
    }
}

/// Allocate a whole source and hand it to a Pole.
pub fn pole(size: usize, shared: bool) -> env::PoleResult<PieToken> {
    if size == 0 || !size.is_multiple_of(env::PAGE_SIZE) {
        return Err(env::make_fail(env::PoleFail::NotAligned));
    }
    let allocation = env::Allocation::new(size, env::PAGE_SIZE)
        .ok_or_else(|| env::make_fail(env::PoleFail::NotAligned))?;
    let reference = env::memory::acquire(
        env::AcquireSource::Allocation(allocation),
        env::PieToken::NONE,
    )
    .map_err(|error| {
        env::make_fail(match error.source {
            env::MemoryFail::OoM => env::PoleFail::OoM,
            env::MemoryFail::Busy => env::PoleFail::Busy,
            env::MemoryFail::NotAligned => env::PoleFail::NotAligned,
            env::MemoryFail::Denied
            | env::MemoryFail::NoRegion
            | env::MemoryFail::AlreadyMapped
            | env::MemoryFail::WidenDenied
            | env::MemoryFail::SourceEmpty
            | env::MemoryFail::DestinationOccupied
            | env::MemoryFail::NotTransferable => env::PoleFail::Denied,
        })
    })?;
    let handed = env::pole::unseal(reference, shared);
    // Pole 已接手这一份来源；无论成败，调用方手里这一份引用都在此结清。
    env::memory::release(reference).expect("fresh caller reference cleanup");
    handed
}

/// 借映共享页，返回映射起点与长度。
pub fn open(token: PieToken) -> env::PoleResult<(usize, usize)> {
    env::pole::open(token).map(|(va, size)| (va.get(), size))
}
