// 运行时基础设施 — 陷阱 trampoline 等运行期原语
//
// 本阶段仅 memory 子系统引用 trampoline 物理地址（`space::init` 映射），
// trampoline 汇编与 stvec 设置待 trap 子系统接入后填充。

pub mod trampoline;
