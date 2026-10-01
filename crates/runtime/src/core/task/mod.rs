//! task — **任务本地原语**：[`join`]（域内并发与结果回收）· [`args`]（启动参数面）·
//! [`heap`]（用户堆后端）· [`lock`]（同域互斥）· [`tls`]（每线程 TLS 块）。

pub mod args;
pub mod heap;
pub mod join;
pub mod lock;
pub mod tls;
