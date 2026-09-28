#![cfg(debug_assertions)]

use crate::hart;

pub fn count() {
    let count = hart::hart_count();
    crate::expect!(
        count > 1,
        "机器只报了 {count} 颗 hart ⇒ 参数表那颗 `-smp 4` 没生效（`QEMU_SMP` 被改过？）。\
         本用例钉的就是那颗默认，与 `scripts/qtest.nu` 的头注同一条"
    );
}