// 窗口 — 映射之上的领域策略适配层（薄：只组合 Space 原语，不持有状态）。
//
// 每种窗口一个类型：分配/映射/回收的**领域语义**（栈 slot 的 guard+body 两 map、
// 帧的立即物化、堆的立即分配、mmap 的懒映射）。方法都取 `&Space`，锁内经
// `SpaceInner` 原语（`alloc`/`register`/`release`/`map`）拼装，产物统一
// [`Span`](super::core::Span)（分配动作的产物 = 回收的输入，类型同一）。
// `Space` 只提供通用映射原语，不知道栈/帧/堆/mmap 是什么。
//
// 窗口 = 零状态（策略命名空间 + 文档锚点）。新增窗口种类 = 加一个类型 + 一组方法，
// `Space` 的 impl、`SpaceInner` 字段零改动。

mod frame;
mod heap;
mod share;
mod stack;

pub(crate) use frame::FrameWindow;
pub(crate) use heap::HeapWindow;
pub(crate) use share::ShareWindow;
pub(crate) use stack::StackWindow;
