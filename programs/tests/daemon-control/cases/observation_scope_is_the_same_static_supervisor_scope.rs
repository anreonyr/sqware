use crate::support::{control, reset};
use crate::system::control::lifecycle::{self, Fail};
use crate::system::control::unit::table::{Slot, State};

#[test]
fn observation_scope_is_the_same_static_supervisor_scope() {
    reset();
    let control = control(true, true, State::Dead, Slot::None);
    let target = lifecycle::authority::targets(&control, crate::TaskId::new(10)).unwrap();
    assert_eq!(target.members, &["worker"]);

    assert!(matches!(
        lifecycle::authority::targets(&control, crate::TaskId::new(99)),
        Err(Fail::Denied)
    ));
    crate::DEAD.with(|dead| dead.borrow_mut().push(crate::TaskId::new(10)));
    assert!(matches!(
        lifecycle::authority::targets(&control, crate::TaskId::new(10)),
        Err(Fail::Denied)
    ));
}
