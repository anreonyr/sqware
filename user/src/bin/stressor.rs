#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use user::PAGE_SIZE;
use user::env::{self, put};

// stressor：用户堆分配器压力测试（有限次、退出收尾——配合 debug 关闭时零泄漏
// 审计）。三幕：
//   1. 多尺寸混合分配/释放（1/2/4/8 页循环）——击打页粒度分合路径；
//   2. 持有-全释放（大块穿插小块）——强制合并、验证闭环后可再分配；
//   3. 失败路径：超大分配必须返回 Err（不 panic），随后普通分配恢复正常。
// 每幕打 '1'/'2'/'3'；全部完成打印 ok 后退出。

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    put("stressor\n").ok();

    // 幕 1：多尺寸混合分配-立即释放
    for i in 0..256usize {
        let size = PAGE_SIZE * (1 << (i % 4));
        let va = env::allocate(size).expect("stressor: alloc 1");
        env::deallocate(va, size).expect("stressor: free 1");
    }
    put("1\n").ok();

    // 幕 2：持有后全释放（强制合并闭环）
    let mut held: Vec<(usize, usize)> = Vec::new();
    for s in [PAGE_SIZE, PAGE_SIZE * 4, PAGE_SIZE * 16, PAGE_SIZE] {
        let va = env::allocate(s).expect("stressor: alloc 2");
        held.push((va, s));
    }
    for (va, s) in held.drain(..) {
        env::deallocate(va, s).expect("stressor: free 2");
    }
    put("2\n").ok();

    // 幕 3：失败路径——超大分配返回 Err 而非 panic；归还后可再分配
    env::allocate(usize::MAX / 2).expect_err("stressor: huge alloc must fail");
    let va = env::allocate(PAGE_SIZE).expect("stressor: re-alloc after oom");
    env::deallocate(va, PAGE_SIZE).expect("stressor: free 3");
    put("3\n").ok();

    put("stressor: ok\n").ok();
    env::exit()
}
