// Control 域（class 6）—— 用户自诊断：Normal 任务回溯采样当前栈，把 pc 数组写进用户 buf。
//
// 与 pie/mail/tole/memory 同形：臂只住本文件，`mod.rs` 只做 decode 与分派。
// 错码是**本域的词汇**（`ControlFail::Denied`）：`buf` 非法（未映射 / 不可写）。

use alloc::sync::Arc;

use env::{ControlCall, ControlFail};

use crate::runtime::diagnose::frame::{self, ResolveCfg, StackReader};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::unit::task::TaskIdent;

use super::ret_err;

/// 本域的臂。**从不挂起**：要么当场答帧数，要么当场答 `ControlFail::Denied`。
pub(super) fn dispatch(frame: &mut TrapContext, call: ControlCall, ident: &Arc<TaskIdent>) {
    match call {
        ControlCall::Backtrace { buf, frames } => {
            // 采样当前任务的栈（`user_satp` 根表，零锁不触缺页），把 pc 数组经
            // `mail::copy_out` 写进用户 buf。
            let world = ident.team.space.kind();
            let sp = frame.gpr.x(Gprs::SP);
            let fp = frame.gpr.x(Gprs::S0);
            let mut reader = StackReader::new(frame.user_satp.ppn());
            let cfg = ResolveCfg::normal(world, sp.saturating_add(frame::SPAN));
            // 域筛：候选 pc 是否属本域代码。符号表已移除：不再做符号命中域筛。
            let code = move |_w: usize| true;
            let (pc_arr, count) = frame::walk(&mut reader, &cfg, sp, fp, Some(&code));
            // 打包 pc 数组字节（仅前 min(count, frames) 帧），copy_out 写用户 buf。
            let keep = count.min(frames);
            let mut bytes = [0u8; frame::DEPTH * core::mem::size_of::<usize>()];
            for i in 0..keep {
                bytes[i * core::mem::size_of::<usize>()..][..core::mem::size_of::<usize>()]
                    .copy_from_slice(&pc_arr[i].pc.as_usize().to_le_bytes());
            }
            let ok = crate::work::mail::copy_out(
                &ident.team.space,
                &bytes[..keep * core::mem::size_of::<usize>()],
                buf,
            );
            // 用户 buf 非法（未映射 / 不可写）⇒ `Denied`；本域只有这一枚失败。
            if ok {
                frame.gpr.set_x(Gprs::A0, keep);
            } else {
                ret_err(frame, ControlFail::Denied);
            }
        }
    }
}
