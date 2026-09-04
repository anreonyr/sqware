#![no_std]
#![no_main]

extern crate alloc;

use user::env::io::put;

// ringer: 1:1 ring 共享内存邮路 demo（v1 未实现跨 Task 共享 Pies；待 G 权利后启用）
#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("ringer: disabled (cross-Task pies deferred to G)\n");
}