use super::super::{Handoff, quit, set_exit_reason};
use crate::core::work::room::conductor::run::run;

pub(super) enum Blocked {
    Resume,
    Switch(Option<usize>),
    Quit(usize),
}

pub(super) fn finish_block(blocked: Blocked) -> Handoff<()> {
    match blocked {
        Blocked::Resume => Handoff::Resume(()),
        Blocked::Switch(next) => Handoff::Switch(next.unwrap_or_else(run)),
        Blocked::Quit(reason) => {
            set_exit_reason(reason);
            Handoff::Switch(quit())
        }
    }
}
