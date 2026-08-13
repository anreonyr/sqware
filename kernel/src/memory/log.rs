// memory 内部日志宏 — 自包含的 no-op 输出
//
// 原依赖内核 macros（debug!/warn! → sbi::write_fmt），现内化于此并经
// `#[macro_use]` 引入 memory 模块作用域（shadow 内核 crate-root 宏）。
//
// debug! 在内核本就 no-op（仅类型检查）；warn! 只在本模块「重复初始化」等
// 罕见路径触发，自包含后一并 no-op——复制 memory 到其他项目无需 SBI 控制台，
// 也无需改动调用点。需要真日志时可在目标项目替换本宏实现。

/// debug 级日志 — no-op（仅类型检查参数，不输出）。
macro_rules! debug {
    ($($arg:tt)*) => {{
        let _ = core::format_args!($($arg)*);
    }};
}

/// warn 级日志 — no-op（原走 SBI；自包含后不输出，不影响逻辑）。
macro_rules! warn {
    ($($arg:tt)*) => {{
        let _ = core::format_args!($($arg)*);
    }};
}
