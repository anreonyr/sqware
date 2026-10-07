pub(crate) fn put(msg: &str) {
    let bytes = msg.as_bytes();
    let _ = env::debug::put(env::VirtAddr::new(bytes.as_ptr() as usize), bytes.len());
}
