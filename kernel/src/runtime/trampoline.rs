// 陷阱 trampoline — 所有地址空间共同映射、共同取指的 trap 入口页
//
// `__alltraps`（保存帧 + 切 satp）与 `__restore`（切回 + 恢复 + sret）位于同一
// 物理页，内核空间与所有用户空间以 TRAMPOLINE VA 映射它（见 manager::TRAMPOLINE）。
//
// 本阶段汇编未接入——`trampoline_pa()` 是占位，`space::init` 映射前必须由
// trap 子系统提供真实物理地址（链接符号或运行时分配的帧）。

/// trampoline 页的物理地址。
///
/// 尚未实现：trap 汇编接入后改为返回链接符号/分配帧的真实地址。
pub fn trampoline_pa() -> usize {
    unimplemented!("trampoline not yet wired up")
}
