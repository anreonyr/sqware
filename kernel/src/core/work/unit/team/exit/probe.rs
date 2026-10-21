pub(super) fn reach(reason: usize) {
    crate::putln!("[critical] guarded exit reason={reason:#x}; no cleanup follows");
}
