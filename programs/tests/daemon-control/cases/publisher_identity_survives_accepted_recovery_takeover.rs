use crate::support::{active, control, open, recheck, recovery, request, reset, tracked};
use crate::system::control::lifecycle::{self, Action, Fail, Origin};
use crate::system::control::unit::table::{Slot, State};
use crate::unit::operator::{Namespace, Placement, Scope};
use system_api::operator::{NamespaceKey, Ops};

#[test]
fn publisher_identity_survives_accepted_recovery_takeover_while_work_is_denied() {
    reset();
    let task = crate::TaskId::new(20);
    let resource = env::Mark::of("takeover/driver");
    let namespace = NamespaceKey::of("service");
    for action in [Action::Recover, Action::Embark { parent: None }] {
        let mut control = control(true, true, State::Ready, Slot::Live { team: None, task });
        control.table.mark_static(task);
        let declarations = Box::leak(vec![Namespace {
            key: namespace,
            placement: Placement::Shared,
            path: "driver",
            scope: Scope::Subtree,
            ops: Ops::BIND,
            permit: None,
            source_mark: Some(resource.get()),
            bindings: &[],
        }].into_boxed_slice());
        control.inputs[1].program = Box::leak(Box::new(crate::unit::UnitFile {
            name: "worker",
            control: crate::unit::ControlConfig { supervision: None, after: None },
            operator: crate::fixtures::operator_config::OperatorConfig { namespace: declarations },
        }));
        assert_eq!(control.publisher(namespace.mark(), resource), Ok(task));
        let mut old = tracked(action, recovery(), Some(task));
        let mut ruin = request(Action::Ruin, Origin::External);
        assert_eq!(lifecycle::takeover::accept(&mut old, &mut ruin), Ok(true));
        lifecycle::intent::accepted(&mut control, "worker", Some(ruin.directive));
        assert_eq!(control.run_requested("worker"), Some(false));
        assert_eq!(control.table.find("worker").unwrap().state, State::Ready);
        assert_eq!(control.publisher(namespace.mark(), resource), Ok(task));
        // Repeat the matching work action: stopped intent must deny either path.
        let mut next = active(action, recovery(), "worker");
        next.0.as_mut().unwrap().progress.execution.task = Some(task);
        assert_eq!(recheck(control, next, open()), Err(Fail::Denied));
    }
}
