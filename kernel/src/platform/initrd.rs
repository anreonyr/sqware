// initrd — 引导期程序清单（**临时机制**）。
//
// 内核侧只剩两件事：
//   1) 定位 initrd 区（`platform::machine::info().initrd`，来自 FDT `/chosen`）；
//   2) 按区里**前 8 字节**取出 **root 镜像**（`plan::manifest::PREAMBLE` 那一格）。
//
// 清单的**解释权在域侧程序**（引导域与编排域各读一遍同一批字节）：内核不含清单格式，
// 只把整区只读映射进引导域空间（VA 由 boot 在 root 的用户段里
// 登记后经启动参数告知）。
//
// 格式（LE，root 侧解析；`build.rs` 打包）：
//   [0..4]   root_off u32      ← **本模块只读这两个数**
//   [4..8]   root_len u32
//   [8..12]  count u32 1..=MAX_PROGRAMS
//   每条：   [u32 kind][u32 name_len][name][u32 len][bytes]
//
// **照实记（这两个数为什么从编译期挪到运行期）**：它们原先按 `env!("ROOT_OFFSET")` 走
// `rustc-env` 由 `build.rs` 回喂 ⇒ 内核的编译单元依赖"打包结果"，于是改任何一个客人都会让
// 内核重编（实测 1.3 s/次）、`cargo build` 也永远不可能只编内核。改读区里这 8 字节之后
// **反馈边断了**；内核仍然不认识清单格式（只读这 8 字节，清单本身归域侧解释）。
//
// 退出路径：正式供给通道（文件服务 / 设备发现）就位后，本模块与 `build.rs` 的
// 打包端一起删除——`Build` 原语本身不随它消失。

/// 取 root 镜像字节（`blob` = initrd 区首，恒等映射下即物理地址）。
///
/// 两个数从区里读（前 8 字节，打包时按引导镜像那一格回填）——**没有编译期常量了**。
pub(crate) fn root_image(blob: &[u8]) -> Option<&[u8]> {
    let off = u32le(blob, 0)? as usize;
    let len = u32le(blob, 4)? as usize;
    blob.get(off..off.checked_add(len)?)
}

/// 读一格 `u32`（LE）；越界 → `None`（不 panic、不猜）。
fn u32le(blob: &[u8], at: usize) -> Option<u32> {
    let s = blob.get(at..at + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
