//! backtrace —— 执行历史的投影核心：walk 的帧 + 地址语义（FrameKind）解析。
//!
//! 领域无关，零分配、不拥有 `Space`、不依赖 `report`/`panic`：回答「执行链是
//! 什么」与「这个地址是什么」。现场采集与渲染在 [`super::scene`]。
//!
//! 与 `frame` 的分工：`frame::walk` 只产出裸地址帧；`FrameKind` 是 resolve 的
//! 产物，不挂在 `Frame` 上——「walk 与 resolve 分离」的类型化。

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::scene::Scene;
use crate::memory::manager::addr::VirtAddr;
use crate::runtime::diagnose::frame::Frame;
use crate::work::unit::space::SpaceKind;

/// 定宽 hex 文本（{:#018x}）——值列的通用形态。
fn hex(x: usize) -> String {
    format!("{x:#018x}")
}

pub(crate) fn symbol(va: VirtAddr) -> String {
    format!("{:#x}", va.as_usize())
}

const DEPTH: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// ROOT 栈区（`[_kernel_edge, +ROOT_STACK_SIZE)`，panic 救援栈）。
    Root,
    /// 内核域（高半区 或 镜像恒等区 `[_kernel_start, _kernel_edge)`）。
    Kernel,
    /// 用户域（分裂位以下，`is_user`）。
    User,
    /// 无法判定（无表 / 域外 / 未知）。
    Unknown,
}

// ── Backtrace — 执行历史投影容器（scene 侧，定长零分配）──────────────

/// 回溯投影 — 执行历史的定长投影（panic 现场**零分配**为类型义务）。
///
/// `frames` 是定长数组（非 `Vec`）——把「回溯层不分配」做成类型约束而非纪律。
/// `Frame`（来自 `frame` 模块）只存裸地址，`FrameKind` 由 [`FrameResolver`] 在
/// assemble 时逐帧另算。
#[derive(Debug)]
pub struct Backtrace {
    frames: [Frame; DEPTH],
    count: usize,
}

impl Backtrace {
    /// 已捕获帧切片（只读）。
    pub(crate) fn frames(&self) -> &[Frame] {
        &self.frames[..self.count]
    }

    /// 由 `frame::walk` 的返回 `(frames, count)` 构造（定长零分配转移所有权）。
    pub(crate) fn from_walk(r: ([Frame; DEPTH], usize)) -> Backtrace {
        Backtrace {
            frames: r.0,
            count: r.1,
        }
    }
}

// ── FrameResolver — 地址语义通道（不拥有 Space）───────────────────────

/// 解析当前域的一套地址语义（`classify`/`executable`）。
///
/// 语义：不问 `&Space`，只持由现场采集层决定的**域归属**（`world`）与回溯表，
/// 据 `VirtAddr::is_kernel/is_user` 与符号表命中判 `FrameKind`。符号表已移除：
/// 地址域归属由 `is_kernel/is_user` 定，规范空洞地址按 Unknown 兜底。
#[derive(Debug, Clone, Copy)]
pub struct FrameResolver {
    world: SpaceKind,
}

impl FrameResolver {
    /// 由 Scene 的现场世界构造（内核/用户域的单一事实源）。
    fn new(world: SpaceKind) -> FrameResolver {
        FrameResolver { world }
    }

    /// 分类：根/内核/用户/未知。
    ///
    /// 三档裁决，`world` 与 `executable` 都参与：
    /// 1. ROOT 栈区（panic 救援栈）→ [`FrameKind::Root`]。
    /// 2. 地址域本身（`is_kernel`/`is_user` → Kernel/User）——`world` 在**域可自定**
    ///    时不作用；仅当地址落在**规范空洞**（既非用户也非内核）才由 `world` 兜底。
    /// 3. 都不中 → 非代码地址（数据指针不足以判执行链）→ [`FrameKind::Unknown`]，
    ///    但若 `executable`（符号表命中）成立则视为本域代码，避免误杀有效的
    ///    `.text` 地址。
    fn classify(&self, pc: VirtAddr) -> FrameKind {
        // ROOT 栈区（panic 救援栈）：[_kernel_edge, +ROOT_STACK_SIZE)。
        let k = crate::machine::kernel_edge();
        if pc.as_usize() >= k && pc.as_usize() < k + crate::layout::ROOT_STACK_SIZE {
            return FrameKind::Root;
        }
        if pc.is_kernel() {
            return FrameKind::Kernel;
        }
        if pc.is_user() {
            return FrameKind::User;
        }
        // 规范的地址本身已能定域；此处不落 `self.world`。
        // 空域（既非用户也非内核）地址：若符号表命中（本域代码）则归本域，否则 Unknown。
        // `self.world` 在域可自定时不作用——本分支只处理 `is_kernel/is_user` 都判不了的
        // 规范空洞，此时以现场世界兜底，避免把有效的镜像恒等区地址判成 Unknown。
        if self.executable(pc) {
            return match self.world {
                SpaceKind::Supervisor => FrameKind::Kernel,
                SpaceKind::User => FrameKind::User,
            };
        }
        FrameKind::Unknown
    }

    /// 该地址是否属本域代码。符号表已移除：无法凭符号命中判定规范空洞地址归属，
    /// 统一按 Unknown 处理（`is_kernel/is_user` 已在上游定域）。
    fn executable(&self, _pc: VirtAddr) -> bool {
        false
    }
}

// ── Scene — 可诊断的执行现场快照（适配层）────────────────────────────

// 历史注：`Registers`（现场寄存器快照：「当前点」，pc/sp/fp 独立于全量 GPR）
// 已迁至 `scene.rs`；本行曾挂着它的文档。

/// [`FrameKind`] 的单字符标签（K 列用；Root/Kernel/User/Unknown → R/K/U/?）。
fn kind_label(k: FrameKind) -> &'static str {
    match k {
        FrameKind::Root => "R",
        FrameKind::Kernel => "K",
        FrameKind::User => "U",
        FrameKind::Unknown => "?",
    }
}

/// 回溯行集（富化格式）：每帧打 `kind | pc hex | sym | space | sp | fp`。
///
/// `Scene` 的 `Frame` 存了 sp/fp/space 语义字段 + [`FrameResolver::classify`] 逐帧
/// 计算的 `kind` —— **富化输出**让这些模型字段真正被消费（而非只打 pc、字段闲置）。
/// 列集的可读性取舍：`kind`（域穿越一眼可见）、`space`（用户态多任务共享空间）、
/// `sp`/`fp`（栈位置）三组语义字段都落列，`pc` 仍是核心定位点。
pub(crate) fn backtrace_rows(scene: &Scene, head: &str) -> Vec<Vec<Option<String>>> {
    let resolver = FrameResolver::new(scene.space);
    let frames = scene.backtrace.frames();
    // 符号化已移除（Team 不再挂符号表）：回溯只显裸地址，无 sym 列。
    let mut rows: Vec<Vec<Option<String>>> = vec![vec![
        Some(head.into()),
        Some("kind".into()),
        Some("pc".into()),
        Some("space".into()),
        Some("sp".into()),
        Some("fp".into()),
    ]];
    for (i, f) in frames.iter().enumerate() {
        let kind = resolver.classify(f.pc);
        rows.push(vec![
            Some(format!("#{i}")),
            Some(kind_label(kind).into()),
            Some(hex(f.pc.as_usize())),
            Some(format!("{:?}", f.space)),
            Some(hex(f.sp.as_usize())),
            Some(
                f.fp.map(|v| hex(v.as_usize()))
                    .unwrap_or_else(|| "-".into()),
            ),
        ]);
    }
    rows
}
