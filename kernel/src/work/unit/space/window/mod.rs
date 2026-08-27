// 窗口 — 动态侧按种类的窗口类型（newtype 包装 [`super::dynamic::Dynamic`]）。
//
// 每种窗口一个类型：构造（几何随类型）、生命周期操作（栈 / 帧：claim/reclaim
// 的 owner 制；堆：allocate/deallocate + mmap/munmap）全部随类型走；`Space`
// 只经 `with` / `with_flush` 锁一次并编排事务（见 `core.rs`）。窗口间共用的
// 区间分配器 / 子 Map 表原语在 `dynamic.rs` 的 [`Dynamic`](super::dynamic::Dynamic) 上。
//
// 新窗口种类 = 新类型 + `SpaceInner` 字段 + `windows()` / `windows_mut()` 登记
// 一处——`Space` 的 impl 零改动。

mod frame;
mod heap;
mod stack;

pub(crate) use frame::FrameWindow;
pub(crate) use heap::HeapWindow;
pub(crate) use stack::StackWindow;