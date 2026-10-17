//! 宿主门：一个阶段一枚字节。
//!
//! 判定权在宿主那一侧：结构化诊断里真看到本阶段该看到的事件（哪个 tid 真入队、
//! 哪条关系真被 signal）之后，门才会开。程序侧只做三件事：把本阶段的事实
//! （阶段名、tid、View 的起点与身份）打成一行控制台、按时限收门、把收尾也报一行。
//! 收不到门就是失败——不睡固定时长、不拿「即将访存」的标志冒充已入队。

use core::time::Duration;

use super::console::Console;

/// 收一门的总时限（纳秒）。宿主那一侧另有自己的时限。
const LIMIT_NS: u64 = 30_000_000_000;

pub struct Gate {
    console: Console,
}

impl Gate {
    pub fn new(console: Console) -> Gate {
        Gate { console }
    }

    /// 报一行本景的话（控制台那一面，宿主按前缀认）。
    pub fn note(&self, text: &str) {
        self.console
            .put(alloc::format!("memory-transfer: {text}\n").as_bytes());
    }

    /// 本阶段就绪：阶段名、View 起点、View 身份与两名故障任务的 tid。
    pub fn ready(&self, phase: &str, va: usize, view: usize, tids: [usize; 2]) {
        self.note(&alloc::format!(
            "phase={phase} ready va={va:#x} view={view} tids={},{}",
            tids[0],
            tids[1]
        ));
    }

    /// 收这一阶段的那一枚字节；到点没等到就是本阶段失败。
    pub fn await_byte(&self, phase: &str, want: u8) {
        let until = env::chrono::clock() + LIMIT_NS;
        loop {
            if let Some(byte) = self.console.get() {
                assert_eq!(byte, want, "memory-transfer: {phase} gate byte");
                return;
            }
            assert!(
                env::chrono::clock() < until,
                "memory-transfer: {phase} gate never arrived"
            );
            execution::room::park(Duration::from_millis(5)).expect("memory-transfer: park");
        }
    }
}
