use crate::support::recovery;
use crate::system::control::lifecycle::{Action, intent};

#[test]
fn intent_policy_keeps_cleanup_and_recovery_out_of_stop_semantics() {
    assert_eq!(intent::change(Action::Mint, lifecycle_origin_external()), Some(false));
    assert_eq!(intent::change(Action::Mint, lifecycle_origin_startup()), Some(false));
    assert_eq!(
        intent::change(Action::Embark { parent: None }, lifecycle_origin_external()),
        Some(true)
    );
    assert_eq!(
        intent::change(Action::Embark { parent: None }, lifecycle_origin_startup()),
        Some(true)
    );
    assert_eq!(
        intent::change(Action::Embark { parent: None }, recovery()),
        None
    );
    assert_eq!(
        intent::change(Action::Debark, lifecycle_origin_external()),
        Some(false)
    );
    assert_eq!(
        intent::change(Action::Ruin, lifecycle_origin_external()),
        Some(false)
    );
    assert_eq!(intent::change(Action::Ruin, lifecycle_origin_reaper()), None);
    assert_eq!(intent::change(Action::Ruin, lifecycle_origin_closing()), None);
    assert_eq!(intent::change(Action::Ruin, lifecycle_origin_rollback()), None);
    assert_eq!(intent::change(Action::Recover, recovery()), None);
}

fn lifecycle_origin_external() -> crate::system::control::lifecycle::Origin {
    crate::system::control::lifecycle::Origin::External
}
fn lifecycle_origin_startup() -> crate::system::control::lifecycle::Origin {
    crate::system::control::lifecycle::Origin::Startup
}
fn lifecycle_origin_reaper() -> crate::system::control::lifecycle::Origin {
    crate::system::control::lifecycle::Origin::Reaper
}
fn lifecycle_origin_closing() -> crate::system::control::lifecycle::Origin {
    crate::system::control::lifecycle::Origin::Closing
}
fn lifecycle_origin_rollback() -> crate::system::control::lifecycle::Origin {
    crate::system::control::lifecycle::Origin::Rollback
}
