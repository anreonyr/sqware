//! Private diagnostics for system clients.

pub(crate) fn put(message: &str) {
    let bytes = message.as_bytes();
    let _ = env::debug::put(env::VirtAddr::new(bytes.as_ptr() as usize), bytes.len());
}
