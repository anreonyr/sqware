use env::wire::FromTriple;
use env::{
    EnvCall, ExitCause, JoinReply, TaskExit, TaskId, TeamId, UnitCall, UnitTarget, Wait, Wire,
};

#[test]
fn join_wire_keeps_target_wait_and_receipt_separate() {
    let call = UnitCall::Join {
        target: UnitTarget::Team(TeamId::new(7)),
        millis: Wait::AtMost(123),
        receive: true,
    };
    assert_eq!(call.slot(), (1usize << 32) | 37);
    assert_eq!(
        UnitCall::from_wire(call.slot(), &call.pack()).unwrap(),
        call
    );
    assert_eq!(
        EnvCall::from_wire(call.slot(), &call.pack()).unwrap(),
        EnvCall::Unit(call)
    );
    let mut invalid = call.pack();
    invalid[3] = 2;
    assert!(UnitCall::from_wire(call.slot(), &invalid).is_err());
}

#[test]
fn retired_slots_and_invalid_targets_are_rejected() {
    for slot in 0..32 {
        assert!(EnvCall::from_wire((1usize << 32) | slot, &[0; 6]).is_err());
    }
    for (tag, id) in [(0, 0), (1, 0), (2, 1), (0, usize::MAX)] {
        assert!(UnitTarget::unpack(&[tag, id, 0, 0, 0, 0], &mut 0).is_err());
    }
}

#[test]
fn nonzero_normal_exit_and_zero_fault_preserve_the_cause() {
    assert_eq!(JoinReply::from_triple(0, 0, 0), JoinReply::Pending);
    for (cause, reason) in [
        (ExitCause::Reap, 9),
        (ExitCause::Fault, 0),
        (ExitCause::Slay, 9),
        (ExitCause::Cascade, 9),
    ] {
        assert_eq!(
            JoinReply::from_triple(cause as usize, 3, reason),
            JoinReply::Reaped(TaskExit {
                task: TaskId::new(3),
                cause,
                reason
            })
        );
    }
}
