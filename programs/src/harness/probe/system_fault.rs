//! Provider-owned system-fault scenario; Source keeps only the probe entry.
pub fn acceptance() {
    system_server::test_support::system_fault::acceptance();
}

pub fn unit() {
    system_server::test_support::system_fault::unit();
}
