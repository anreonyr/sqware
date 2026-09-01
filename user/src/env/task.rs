//! Task 域：`TaskCall::*` 转发。

use ubi::{TaskCall, UArgs, UResult, Ucall, UcallBuilder};

pub fn spawn(entry: usize, arg: usize, stack: usize) -> UResult<usize> {
    let args = UArgs { a0: entry, a1: arg, a2: stack, ..UArgs::default() };
    let (v0, _) = UcallBuilder::new(Ucall::Task(TaskCall::Spawn)).args(args).call()?;
    Ok(v0)
}