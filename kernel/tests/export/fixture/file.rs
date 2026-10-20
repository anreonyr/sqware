use std::io::{self, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub static CREATE_FAIL: AtomicBool = AtomicBool::new(false);
pub static CREATE_CALLS: AtomicUsize = AtomicUsize::new(0);
pub static WRITE_FAIL_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
pub static WRITE_CALLS: AtomicUsize = AtomicUsize::new(0);
pub static BYTES: Mutex<Vec<u8>> = Mutex::new(Vec::new());

pub struct File;

impl File {
    pub fn create(_: &std::ffi::CStr) -> io::Result<Self> {
        CREATE_CALLS.fetch_add(1, Ordering::Relaxed);
        if CREATE_FAIL.load(Ordering::Relaxed) {
            Err(io::Error::other("injected create failure"))
        } else {
            Ok(Self)
        }
    }
}

impl Write for File {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let call = WRITE_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
        if call == WRITE_FAIL_AT.load(Ordering::Relaxed) {
            return Err(io::Error::other("injected write failure"));
        }
        BYTES.lock().unwrap().extend_from_slice(bytes);
        std::thread::yield_now();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
