use crate::startup::Startup;

#[test]
fn initial_denial_retries_until_the_fixed_window_expires() {
    let startup = Startup::new(100);
    assert!(!startup.deny_is_fatal(99));
    assert!(startup.deny_is_fatal(100));
}

#[test]
fn denial_after_a_valid_observation_is_always_fatal() {
    let mut startup = Startup::new(u64::MAX);
    startup.observed();
    assert!(startup.deny_is_fatal(0));
}
