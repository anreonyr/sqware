// initrd — 引导期程序清单（**临时机制**）。
//
// 内核侧只剩两件事：
//   1) 定位 initrd 区（`machine::info().initrd`，来自 FDT `/chosen`）；
//   2) 按打包期常量取出 **root 镜像**（`ROOT_OFFSET`/`ROOT_LEN`）。
//
// 清单的**解释权在 root 域程序**（`programs/src/bin/supervisor/root/manifest.rs`）：
// 内核不含清单格式，只把整区只读映射进 root 空间（VA 由 boot 在 root 的用户段里
// 登记后经启动参数告知）。见 `docs/root.md`。
//
// 格式（LE，root 侧解析；`build.rs` 打包）：
//   [0..4] count u32 1..=MAX_PROGRAMS
//   每条：[u32 kind][u32 name_len][name][u32 len][bytes]
//
// 退出路径：正式供给通道（文件服务 / 设备发现）就位后，本模块与 `build.rs` 的
// 打包端一起删除——`Build` 原语本身不随它消失。

/// root 镜像在 initrd blob 内的字节偏移（`build.rs` 打包时经 rustc-env 导出）。
pub(crate) const ROOT_OFFSET: usize = parse_usize(env!("ROOT_OFFSET").as_bytes());
/// root 镜像长度。
pub(crate) const ROOT_LEN: usize = parse_usize(env!("ROOT_LEN").as_bytes());

/// 编译期十进制解析（`env!` 只给 `&str`，`usize::from_str` 非 const）。
const fn parse_usize(s: &[u8]) -> usize {
    let mut v = 0usize;
    let mut i = 0;
    while i < s.len() {
        v = v * 10 + (s[i] - b'0') as usize;
        i += 1;
    }
    v
}

/// 取 root 镜像字节（`blob` = initrd 区首，恒等映射下即物理地址）。
pub(crate) fn root_image(blob: &[u8]) -> Option<&[u8]> {
    blob.get(ROOT_OFFSET..ROOT_OFFSET + ROOT_LEN)
}
