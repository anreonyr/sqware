// 镜像源 — 一份字节从哪来。
//
// `parser` / `loader` / `Spawn` 三条路共用这一个源结构：镜像的头、段表、段实体，
// 以及 `Spawn` 的启动参数，说的都是同一件事——"从某个源里读一段字节"。
//
// 两个变体 = 两种来路：内核自己读得到的（boot 的恒等映射 initrd），与**别的地址空间**
// 里的一段（envcall 的调用方）。后者**不保留物理地址**：`read` 每次现翻页表，故"源被
// 放手之后还留着一枚悬垂的 PA"这件事在这里表达不出来（没有那个态）。

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;

use super::space::Space;

/// 读头窗口：`Build` 的镜像头与段表必须落在文件前 `HEAD` 字节内。
///
/// **照实记（这条界有多紧）**：ELF 头 64 B + 段表 `phnum × 56`。实测 62 张镜像
/// （`target/image/{release,debug}`）恒为 `phoff(64) + 5 × 56 = 344 B`；`HEAD` 容得下
/// `phnum ≤ 72`，今天的 5 条占 2 格。
///
/// **撞上去是什么样**：`parser` 答 `Truncated` ⇒ `Build` 答 `BadImage`（−6），编排者
/// 的失败单上写的是"镜像不认"——而真相是"段表不在文件前 4 KiB 内"。加 `PT_NOTE` /
/// `PT_GNU_PROPERTY` 这类段把 `phnum` 推过 72 时会看见这一格。
///
/// 载体**在堆上**（一页），不在 trap 栈上——栈宝贵。
pub(crate) const HEAD: usize = PAGE_SIZE;

/// 镜像源（`parser` / `loader` / `Spawn` 共用的唯一源结构）。
#[derive(Clone, Copy)]
pub(crate) enum Source<'a> {
    /// 内核可直接读的一块（boot：恒等映射的 initrd 区）。
    Slice(&'a [u8]),
    /// 另一个地址空间里的一段（envcall：调用方给的 VA + 长度）。
    Space {
        space: &'a Space,
        va: VirtAddr,
        /// 这一段的字节数（**调用方声明**；`read` 的上界）。
        len: usize,
    },
}

impl Source<'_> {
    /// 源的长度（字节）。
    pub(crate) fn len(&self) -> usize {
        match self {
            Source::Slice(bytes) => bytes.len(),
            Source::Space { len, .. } => *len,
        }
    }

    /// 从源内偏移 `off` 起读满 `dst`。
    ///
    /// - 全部读到 → `true`；**否则 `false`，且 `dst` 的内容未定义**（可能只写了一半）。
    ///   与 `mail::copy_in` 那条"要么全读、要么一个字节都不动"**不同**，是明知的选择：
    ///   本条路上 `false` 的两个调用点都当场弃掉整个 `dst`（帧随 `Vec` drop、头窗口随
    ///   栈帧归还），故全或全不动的强契约在这里买不到东西，却要多走一趟页表。
    /// - `false` 有两条来路——越界（`off + dst.len() > len`）与区间内有页没映射——
    ///   **同答 `false`**：调用方对两者的动作相同（`Denied`），拆开只多一个没人在场的
    ///   判别值。
    /// - `dst` 为空 → `true`（空读恒成立，故 `count == 0` 没有特例）。
    /// - 前置：**无**。任意 `off` / `dst` 都安全、**不 panic**（越界与 VA 相加都走
    ///   checked 算术，溢出同落 `false`）。
    pub(crate) fn read(&self, off: usize, dst: &mut [u8]) -> bool {
        let Some(end) = off.checked_add(dst.len()) else {
            return false;
        };
        if end > self.len() {
            return false;
        }
        match self {
            Source::Slice(bytes) => {
                dst.copy_from_slice(&bytes[off..end]);
                true
            }
            Source::Space { space, va, .. } => {
                // **先规范、再查、再交给 `segments`**：`from_raw` 会把分裂位以上的地址
                // 一并进位（高位地址因此可能贴到 `usize::MAX`），而 `Space::segments`
                // 构造时要算 `at + len`（未 check）。`va` 与 `len` 都是调用方可控的，
                // 这两步漏一步就是一条用户可触的溢出 panic。
                let Some(raw) = va.as_usize().checked_add(off) else {
                    return false;
                };
                let at = VirtAddr::from_raw(raw);
                if at.as_usize().checked_add(dst.len()).is_none() {
                    return false;
                }
                // 逐页翻译的现成原语（`space::outer`）：遇未映射即停，正是要的语义。
                let mut done = 0usize;
                for (pa, _flags, chunk) in space.segments(at, dst.len()) {
                    // SAFETY: `pa` 为恒等映射的物理地址（`space.segments` 的产出）；
                    // 本段已翻译到帧，且 `chunk` 落在 `[at, at + dst.len())` 内。
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            pa.as_usize() as *const u8,
                            dst.as_mut_ptr().add(done),
                            chunk,
                        );
                    }
                    done += chunk;
                }
                done == dst.len()
            }
        }
    }
}
