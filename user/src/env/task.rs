//! Task 域：`TaskCall::*` 转发。

use ubi::{TaskCall, TaskCallRet, EnvResult};

pub fn spawn(entry: usize, arg: usize, stack: usize) -> EnvResult<usize> {
    let r = TaskCall::Spawn { entry, arg, stack }.call()?;
    match r {
        TaskCallRet::Spawn(id) => Ok(id.get()),
        _ => unreachable!(),
    }
}
