use super::{Action, Origin};
use crate::system::control::unit::Control;

/// Intent change implied by an accepted action.
///
/// `None` keeps the existing intent: recovery, natural reaping, closing and
/// failure rollback are not explicit stop requests.
pub(crate) fn change(action: Action, origin: Origin) -> Option<bool> {
    match action {
        Action::Mint
            if matches!(origin, Origin::External | Origin::Startup) =>
        {
            Some(false)
        }
        Action::Embark { .. } if matches!(origin, Origin::External | Origin::Startup) => {
            Some(true)
        }
        Action::Debark | Action::Ruin if matches!(origin, Origin::External) => Some(false),
        _ => None,
    }
}

/// Commit the intent effect of an accepted admission. Only an explicit Ruin
/// that reused an in-flight job is committed here; a fresh action is committed
/// by its own validated stage.
pub(crate) fn accepted(
    control: &mut Control,
    name: &str,
    reused: Option<super::Directive>,
) {
    let Some(directive) = reused else {
        return;
    };
    if directive.action == Action::Ruin && directive.origin == Origin::External {
        accept_ruin_stop(control, name);
    }
}

fn accept_ruin_stop(control: &mut Control, name: &str) {
    if control.run_requested(name).is_some() {
        let _ = control.request_stop(name);
    }
}
