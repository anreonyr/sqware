#![no_std]
#![no_main]

use task::env::io::put;

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("C\n");
}
