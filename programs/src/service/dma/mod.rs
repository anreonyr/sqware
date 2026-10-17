//! 端点服务那一台（`system-dma`）：**跨 Team** 的真任务，只领一枚精确到一根 BAR
//! 的窗口与几个 IOVA。声明在 `program.rs`，身子在 `endpoint.rs`。

pub mod endpoint;
