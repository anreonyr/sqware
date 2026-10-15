use core::sync::atomic::{AtomicU8, Ordering};

use super::error::FlatError;

const SOLE: u8 = 0;
const ALIASED: u8 = 1;
const IN_TRANSIT: u8 = 2;

/// 一批承载的登记：逐个登记，碰上任一正在在途的就整批失败。
///
/// 已经登记过的不回退（登记是永久效果），所以重试同一批是幂等的。
pub(super) fn register<'a>(states: impl IntoIterator<Item = &'a Custody>) -> Result<(), FlatError> {
    for state in states {
        state.register()?;
    }
    Ok(())
}

/// 一个 `Frame` 的迁移托管：只归一个来源槽、已被别的 `Flat` 登记、或正被一次移交占用。
///
/// `from_frames` 导入把它从 `Sole` 变成 `Aliased`（幂等、不可逆），
/// 于是同一份帧不可能再从原来源迁出。导入与移交准备争的是**同一个**原子，
/// 不用 `Arc` 强计数当独占证明，也没有“先裸读、另按一面旗”的缺口。
#[derive(Debug)]
pub(super) struct Custody(AtomicU8);

impl Custody {
    pub(super) const fn new() -> Self {
        Self(AtomicU8::new(SOLE))
    }

    /// 已经被别的 `Flat` 登记：不再可迁移，且不可逆。
    pub(super) fn is_aliased(&self) -> bool {
        self.0.load(Ordering::Acquire) == ALIASED
    }

    /// 取得移交独占：只有 `Sole` 能变成 `InTransit`。
    pub(super) fn enter(&self) -> bool {
        self.0
            .compare_exchange(SOLE, IN_TRANSIT, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// 移交结束（提交或回滚）：独占还给来源槽状态。
    ///
    /// 只有从 `InTransit` 离开是合法迁出；其余状态到这里就是内部不变量坏了。
    pub(super) fn leave(&self) {
        assert!(
            self.0
                .compare_exchange(IN_TRANSIT, SOLE, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "transfer custody leaves only from InTransit"
        );
    }

    /// 这份帧被别的 `Flat` 登记（`from_frames` 导入）：已登记过是幂等的，
    /// 正被一次移交占用时暂不可登记。
    pub(super) fn register(&self) -> Result<(), FlatError> {
        loop {
            match self.0.load(Ordering::Acquire) {
                ALIASED => return Ok(()),
                IN_TRANSIT => return Err(FlatError::Busy),
                _ => {
                    if self
                        .0
                        .compare_exchange(SOLE, ALIASED, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        return Ok(());
                    }
                }
            }
        }
    }
}
