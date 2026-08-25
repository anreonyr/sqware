//! export —— 诊断宿主导出（semihosting fs → 单文件 JSON 流）。
//!
//! 两条 JSON 行流：事件行（实时逐条 [`push`]）+ 报告快照行（整档 [`export`]
//! 一次追加）。消费端按行解析：事件行 = 小对象（含 kind），报告行 = 大对象
//! （含 paras）。
//!
//! 序列化全交 serde_json（零手写 JSON）；跨核写入经 HOST 锁 try_lock 串行，
//! 失败闩静默停用（尽力而为，诊断路径不阻塞）。

use core::sync::atomic::{AtomicBool, Ordering};

use semihosting::io::Write as _;

use crate::lock::SpinLock;
use crate::runtime::diagnose::report::Report;

/// 导出文件名。
const EXPORT_NAME: &core::ffi::CStr = c"sqware-diagnose.jsonl";

/// 打开失败闩：告警一次后静默停用（避免每记录重试 open 刷屏）。
static BROKEN: AtomicBool = AtomicBool::new(false);

/// HOST 锁等待上限（ticks；timebase 10MHz 下 ≈1ms）：持锁方写一条记录在该
/// 量级内必完成；超限即放弃（诊断路径不可死等）。
const HOST_WAIT_TICKS: u64 = 10_000;

/// 拿锁 + 确保文件 + 执行写（尽力而为：超时跳过、broken 闩静默、写失败不回读）。
/// 文件句柄与锁同居（`SpinLock<Option<File>>`）：一次持锁完成「取句柄 →
/// 写」，不再需要第二层 OnceLock 的可变借用。
fn with_host(f: impl FnOnce(&mut semihosting::fs::File)) {
    static HOST: SpinLock<Option<semihosting::fs::File>> = SpinLock::new(None);
    let deadline = crate::runtime::chrono::clock::now()
        .as_ticks()
        .wrapping_add(HOST_WAIT_TICKS);
    let mut g = loop {
        if let Some(g) = HOST.try_lock() {
            break g;
        }
        if crate::runtime::chrono::clock::now().as_ticks() >= deadline {
            return; // 超时：静默跳过（尽力而为，不阻塞诊断路径）
        }
        core::hint::spin_loop();
    };
    if BROKEN.load(Ordering::Relaxed) {
        return;
    }
    if g.is_none() {
        match semihosting::fs::File::create(EXPORT_NAME) {
            Ok(f) => *g = Some(f),
            Err(_) => {
                BROKEN.store(true, Ordering::Relaxed);
                crate::putln!(
                    "semihosting: cannot create {:?}; host export disabled",
                    EXPORT_NAME
                );
                return;
            }
        }
    }
    let file = g.as_mut().expect("just stored above");
    f(file);
}

/// 推一条 JSON 行（`json` 须为合法 JSON 文本）。
pub fn push(json: &[u8]) {
    with_host(|f| {
        let _ = f.write_all(json);
        let _ = f.write_all(b"\n");
    });
}

/// 整档报告快照：一次 `to_vec` 追加一行（序列化失败静默空——诊断不 panic）。
pub fn export(r: &Report) {
    let json = serde_json::to_vec(r).unwrap_or_default();
    push(&json);
}
