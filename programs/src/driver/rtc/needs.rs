//! rtc::needs — **本域自己那片硬件账**：要哪一类设备、什么种类/权/形态。
//!
//! 它住在本域里，因为它是**收方**开的那张单子：装配者照它开单（[`WANTS`] 那几条原样递出去），
//! 本域收到记录后**按位次归位**（[`crate::driver::assemble::receive`]——位置即格）。
//!
//! **写的是类，不是名字**：`google,goldfish-rtc` 是这台设备的绑定名（树里的 `compatible`），
//! 而"这一类是哪一台"由编排域读树定下来——**本域不发明名字，也不冻机器地址**。

use protocol::driver::supply::call::{Kind, Need, class_block};
use runtime::core::port::{Access, Policy};

/// 本域要的那一枚 —— **直接就是单子上的那一条**。
///
/// 类取 `google,goldfish-rtc`（这台实时钟的绑定名）。
///
/// `ONLY`（独占）是**资源事实**：一页寄存器同一时刻只该有一个持有者——读时间、写闹钟、清状态
/// 都是本域在做，故这一枚由**本域**从装配者手里领。权位要 `STORE`：本域真的写那几格。
pub const WANTS: &[Need] = &[Need::class(
    class_block("google,goldfish-rtc"),
    Kind::Pole,
    Access::FETCH_STORE,
    Policy::ONLY,
)];
