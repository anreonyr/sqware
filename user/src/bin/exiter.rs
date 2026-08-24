#![no_std]
#![no_main]

use user::env::put;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let _ = put("C\n");
    user::env::exit()
}
