// debug — 调试面（class 8）：域直接借内核的 DBCN 打印/读入。
//
// 为什么内核要有这一格：**引导期服务全都不存在**——root 起 dir/plic/uart/console 的
// 那一段里，"哪一步算不下去"只有域自己知道，而它连一句 `say` 都递不出去（控制台服务
// 还没上线）。内核自己的打印（`crate::putln!`）走 SBI DBCN，这一格就是把它**借给域**。
//
// 它与 `docs/driver.md` §10「设备不再是内核的事」不冲突：这里不碰设备、不认 UART，
// 走的是固件的调试控制台（内核自己的出口）。设备写仍归持设备者。
//
// 不设构建门：见 `env::fid::DebugCall` 那条理由（嵌套构建的 `debug_assertions` 会分叉）。

use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::putln;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::unit::gate::GateError;
use crate::work::unit::task::TaskIdent;

/// 一次能搬的字节数上限（与 `env::DBCN_MAX` 同值：栈上定长，不分配）。
const DBCN_MAX: usize = 256;

/// 域 → 调试控制台。
///
/// 逐字节 `read_volatile`：域那段地址可能不在恒等区（用户窗口），而 `putln!` 只认
/// 内核自己的地址（非恒等区走页表译，用户地址一律静默丢）。故先把字节**搬进内核的
/// 栈缓冲**，再交给既有出口打印。至多 [`DBCN_MAX`] 字节，多出的截断。
pub(super) fn put(
    frame: &mut TrapContext,
    ident: &TaskIdent,
    buf: usize,
    len: usize,
) -> *mut TrapContext {
    if len == 0 {
        return err(frame, GateError::Denied);
    }
    let n = len.min(DBCN_MAX);
    let mut text = [0u8; DBCN_MAX];
    let space = &ident.team.space;
    for (i, slot) in text[..n].iter_mut().enumerate() {
        let at = KVirt::from_raw(buf.wrapping_add(i));
        let Some((pa, _)) = space.translate(at) else {
            return err(frame, GateError::Denied);
        };
        // SAFETY: `translate` 已把该 VA 落到一个有效物理页；恒等映射区 PA 可直读。
        *slot = unsafe { core::ptr::read_volatile(pa.as_usize() as *const u8) };
    }
    let text = core::str::from_utf8(&text[..n]).unwrap_or("<non-utf8>");
    putln!("{text}");
    frame.gpr.set_x(Gprs::A0, n);
    frame
}

/// 调试控制台 → 域：内核栈暂存接一次 DBCN 读，再把结果写进域。
///
/// **可能阻塞**：DBCN 的 console read 语义是"等到至少读到一个字节"。单核上会挂住
/// 整机，多核只挂住调用它的那一核——敢不敢在这儿等，是调用方的判断（见
/// `env::fid::DebugCall::Get` 的注）。
pub(super) fn get(
    frame: &mut TrapContext,
    ident: &TaskIdent,
    buf: usize,
    len: usize,
) -> *mut TrapContext {
    if len == 0 || len > DBCN_MAX {
        return err(frame, GateError::Denied);
    }
    let mut stage = [0u8; DBCN_MAX];
    let n = match crate::console::read(&mut stage[..len]) {
        Some(n) if n <= len => n,
        // 固件给不出这一格（非 DBCN / 短读）：如实拒，不假装成功。
        _ => return err(frame, GateError::Denied),
    };
    if n == 0 {
        frame.gpr.set_x(Gprs::A0, 0);
        return frame;
    }
    let space = &ident.team.space;
    if !crate::work::mail::copy_out(space, &stage[..n], buf) {
        return err(frame, GateError::Denied);
    }
    frame.gpr.set_x(Gprs::A0, n);
    frame
}

fn err(frame: &mut TrapContext, e: GateError) -> *mut TrapContext {
    frame.gpr.set_x(Gprs::A0, e.code() as usize);
    frame
}
