#[derive(Clone)]
pub(crate) enum WaitCall {
    Join(crate::core::work::unit::join::JoinWait),
    Mail(crate::core::work::mail::tole::AwaitWait),
}
