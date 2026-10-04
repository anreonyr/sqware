//! res — 四种资源的厚壳（配对那一层，不是往返那一层）：
//! [`port`]（Hole：授出/坐标/一条会话）· [`dock`]（Pole：借映成视图）· [`bell`]（Nole：门铃）·
//! [`pile`]（Tole：几枚可等地挂到一处）。
//!
//! [`pie`] 是它们共用的**薄句柄**（四枚门闩 ＋ `AnyPie` / `Mate`）：厚壳持一枚薄句柄再加
//! 一样东西，薄句柄管"这一枚怎么用"。

pub mod bell;
pub mod dock;
pub mod pie;
pub mod pile;
pub mod port;
