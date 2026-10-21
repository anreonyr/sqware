pub(super) fn stop(reason: usize) -> ! {
    #[cfg(feature = "critical-scene")]
    super::probe::reach(reason);
    crate::entry::report::halt::protected_stop(reason)
}
