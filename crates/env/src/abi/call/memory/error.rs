//! Memory 域（class 2：用户堆与映射）的失败词汇。

use mold::Fail;

/// Memory 域（class 2）的失败词汇；码是线与内核共用的那一条轴，
/// 新调用的失败不另起域、不换码。
#[derive(Fail)]
pub enum MemoryFail {
    /// 句柄不属于调用者、访问位非法，或来源/目标控制未获授权。
    Denied = -1,
    /// 段耗尽 / 物理帧耗尽 / 映射表备不下。
    OoM = -2,
    /// 地址、范围坐标或选址描述不满足格式与对齐约束。
    NotAligned = -3,
    /// 这一段不在任何已声明的映射里。
    NoRegion = -4,
    /// 该 VA 已被映射。
    AlreadyMapped = -5,
    /// 借入页（别人的所有权）不许加宽。
    WidenDenied = -6,
    #[busy]
    Busy = -7,
}

/// `MemoryFail` 的结果别名。
pub type MemoryResult<T> = Result<T, erra::Error<MemoryFail>>;
