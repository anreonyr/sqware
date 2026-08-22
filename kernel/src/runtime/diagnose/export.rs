//! export — 诊断宿主导出（semihosting fs → JSON Lines 单文件）。
//!
//! diagnose 族唯一「导出到宿主文件」的设施：trace/scene/halt/watch 的结构化
//! 记录都经本模块写进宿主文件 sqware-diagnose.jsonl（JSON Lines，自描述长度
//! "len" = 整行对象字节数，含自身）。终端纯文本归 console.rs——本模块不碰终端。
//!
//! 设计决策：
//!   - **单文件全局流**：lazy create+append；首行 '#' 头部溯源只写一次。
//!   - **崩溃现场可用**：跨核 try_lock 串行（拿不到即跳过、不阻塞），零 panic、
//!     零分配；打开失败告警一次后静默停用（闩）。
//!   - **自描述长度**：每条记录 `{<fields>,"len":N}`，N 由字段求不动点；
//!     消费端按 len 校验/丢弃被截断或交错的半条（栈缓冲满即截断，不 panic）。
//!   - **命名共享**：事件 kind 与模块记录 kind 共用命名空间（trace 的 "panic"/
//!     "watch" 事件与 halt/watch 模块记录同名）——消费端按字段区分：事件记录
//!     只有 h/t/kind/len，模块记录带 msg/report 等字段。
//!
//! 依赖 QEMU -semihosting（feature 默认开启，runner 恒加该标志）。

use core::fmt;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::lock::{OnceLock, SpinLock};

/// 导出文件名。QEMU semihosting fs 按 **qemu 进程 CWD**（= 调用 runner 的目录）
/// 解析相对路径；runner 结束后归档为 diagnose-<seed>-<ts>.jsonl。
const EXPORT_NAME: &core::ffi::CStr = c"sqware-diagnose.jsonl";

/// 打开的文件句柄（lazy：首次写时 create，即截断 qemu CWD 下的旧导出）。
static FILE: OnceLock<semihosting::fs::File> = OnceLock::new();
/// 头部已写（首行溯源）——只写一次。
static STARTED: AtomicBool = AtomicBool::new(false);
/// 打开失败闩：告警一次后静默停用（避免每记录重试 open 刷屏）。
static BROKEN: AtomicBool = AtomicBool::new(false);

/// 写一条结构化记录：`{<fields>,"len":N}\n`。
///
/// fields 闭包向 [`Buf`] 写记录字段（含 `"h"`/`"t"`/`"kind"` 与模块自有字段；
/// 文本字段经 [`json_esc`]）。框架自动补 `,"len":N` 与首行 '#' 头部。
/// 尽力而为：try_lock 失败（panic 瞬间他核正写）或 broken 闩 → 静默跳过；
/// 记录超过栈缓冲 → 截断（消费端按 len 不符丢弃）。诊断路径永不 panic。
pub fn line(fields: impl FnOnce(&mut Buf)) {
    use semihosting::io::Write as _;
    static HOST: SpinLock<()> = SpinLock::new(());
    if let Some(_g) = HOST.try_lock() {
        if BROKEN.load(Ordering::Relaxed) {
            return;
        }
        if FILE.get().is_none() {
            match semihosting::fs::File::create(EXPORT_NAME) {
                Ok(f) => {
                    let _ = FILE.set(f);
                }
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
        let mut file = FILE.get().expect("just stored above");
        if !STARTED.swap(true, Ordering::Relaxed) {
            let mut head = [0u8; 512];
            let n = header(&mut head);
            let _ = file.write_all(&head[..n]);
            let _ = file.write_all(b"\n");
        }
        let mut out = [0u8; 560];
        let n = record(fields, &mut out);
        let _ = file.write_all(&out[..n]);
        let _ = file.write_all(b"\n");
    }
}

/// JSON 字符串转义：把 s 原样写入 w，遇 `"` `\` 与 <0x20 控制字符加反斜杠转义。
/// 用于 halt/watch 的文本字段（loc/msg/task/report）——事件字段全是标量，无需转义。
pub fn json_esc(w: &mut Buf, s: &str) -> fmt::Result {
    for c in s.chars() {
        match c {
            '"' => w.write_str("\\\"")?,
            '\\' => w.write_str("\\\\")?,
            '\n' => w.write_str("\\n")?,
            '\r' => w.write_str("\\r")?,
            '\t' => w.write_str("\\t")?,
            c if (c as u32) < 0x20 => write!(w, "\\u{:04x}", c as u32)?,
            c => w.write_char(c)?,
        }
    }
    Ok(())
}

/// 组装一条记录：'{' + fields + ',"len":N}'. 返回写入 out 的字节数（截断即停）。
/// len 与 total_len 不动点一致（total = used + 9 + digits(total)）。
fn record(fields: impl FnOnce(&mut Buf), out: &mut [u8]) -> usize {
    let mut body = [0u8; 512];
    let mut used = 0usize;
    {
        let mut b = Buf(&mut body, &mut used);
        fields(&mut b);
    }
    let total = total_len(used);
    let mut o = 0usize;
    {
        let mut w = Buf(out, &mut o);
        let _ = w.write_str("{");
        let _ = w.write_str(core::str::from_utf8(&body[..used]).unwrap_or(""));
        let _ = write!(w, ",\"len\":{total}}}");
    }
    o
}

/// total = '{' + body + ',"len":'(7) + digits(total) + '}' = used + 9 + digits(total)。
/// 求不动点（digits 单调，1~2 步收敛）。
fn total_len(used: usize) -> usize {
    let mut total = used + 9 + 1;
    loop {
        let d = digits(total);
        let want = used + 9 + d;
        if want == total {
            return total;
        }
        total = want;
    }
}

fn digits(mut n: usize) -> usize {
    let mut d = 1usize;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

/// 头部一行：# sqware semihosting t=<时期秒>。以 '#' 开头，消费端跳过非 '{' 行。
/// 时刻用 SYS_TIME 的 Result API（尽力而为），不用 experimental::time 的
/// SystemTime::now —— 其内部 unwrap，违反诊断路径不 panic。只依赖 fs feature。
fn header(out: &mut [u8]) -> usize {
    let mut o = 0usize;
    {
        let mut w = Buf(out, &mut o);
        let _ = w.write_str("# sqware semihosting");
        match semihosting::sys::arm_compat::sys_time() {
            Ok(secs) => {
                let _ = write!(w, " t={secs}");
            }
            Err(_) => {
                let _ = w.write_str(" t=?");
            }
        }
    }
    o
}

/// 栈上写缓冲（格式化成行再整段 write，避免逐字符 SBI 调用）。满即截断，不 panic。
pub struct Buf<'a>(&'a mut [u8], &'a mut usize);

impl fmt::Write for Buf<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let used = *self.1;
        let n = s.len().min(self.0.len().saturating_sub(used));
        self.0[used..used + n].copy_from_slice(&s.as_bytes()[..n]);
        *self.1 = used + n;
        Ok(())
    }
}