#![no_std]
#![no_main]

use user::env::put;

// 共享入口在 user::entry：_start 引导 + panic 处理；这里只需定义 main。
#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 对照旧 demo program_c：写 'C' 后退出，验证 parser→loader→TaskBuilder 全链
    let _ = put(b'C');
    user::env::exit()
}
