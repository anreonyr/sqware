//! dock — **Pole 的 resource 封装**：把一枚共享页借映进本域，得到一段视图。
//!
//! 内核里它**仍是一枚 Pole**（没有新资源）：页块与 `Open`/`Shut`/`Narrow`
//! 都在内核，"怎么用"封装在这一层。与 `Port` 包着私有孔句柄同构：**种类归内核，用法归
//! resource**。
//!
//! 三件各包一种 primitive，各加一样东西：
//!
//! ```text
//! Port   两枚 Hole   加**配对**   往哪推、从哪收、对面是谁
//! Dock   一枚 Pole   加**成对**   起点与长度成对的一段内存
//! Bell   一枚 Nole   加**约束**   三拍 ring / wait / hush（只有一条方向）
//! ```
//!
//! # 为什么视图是**另一个类型**
//!
//! 映射与视图的可复制性相反：映射不该被复制（`shut` 要消费它），而视图**必须**能被复制
//! ——`static UART` 配 `Uart: Copy`，两个线程各取一份（同一张页表，不需要锁）。故两个类型：
//! [`Dock`] 不 `Copy`，[`View`] 是 `Copy` 的值。
//!
//! # 映射与撤图
//!
//! `open` / `shut` 对应 `PoleCall::Open` / `Shut`；本层增加映射的持有责任：
//! 把"起点 + 长度"成对地带回来。于是"一段不知道多长的内存"在类型上不存在。
//!
//! # 门闩收下，但不代管它的收尾
//!
//! `shut` 只撤图，**不 `release` 门闩**——与 `Port::shut` 同一条理由：门闩的收尾是策略，
//! 不进结构。

pub use crate::view::View;

/// 默认后端下的泊位。
pub type Dock = crate::mapping::Dock<crate::native::Native>;
