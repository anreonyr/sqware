#![no_std]
#![no_main]

use user::env::io::put;

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("C\n");
}
