use core::panic::PanicInfo;
use rustsbi::Console;

#[panic_handler]
fn panic_handler(_info: &PanicInfo) -> ! {
    let s = b"fuck off";
    loop {}
}
