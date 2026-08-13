// memory 内部平台层 — 自包含的内存区域注入
//
// 原依赖内核 platform（DTB 探测 DRAM 配置），现由调用方在 `allocator::init`
// 时显式注入物理内存池区域——复制 memory 到其他项目无需内核 platform。
//
// Config 字段名与内核 platform::Config 保持一致，使 frame.rs/block.rs 的
// `cfg.dram_base .. +dram_size` debug 越界检查语义不变（即注入的 [base, end)）。

use crate::lock::OnceLock;

/// 内存池配置
#[derive(Clone, Copy)]
pub(crate) struct Config {
    pub dram_base: usize,
    pub dram_size: usize,
    pub hart_count: usize,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

/// 注入内存池区域与 hart 数（`allocator::init` 调用，恰好一次）
pub(crate) fn init(region: crate::memory::allocator::Region, hart_count: usize) {
    let _ = CONFIG.set(Config {
        dram_base: region.base,
        dram_size: region.end - region.base,
        hart_count,
    });
}

/// 读取注入的配置
pub(crate) fn get() -> &'static Config {
    CONFIG
        .get()
        .expect("memory platform config not initialized")
}
