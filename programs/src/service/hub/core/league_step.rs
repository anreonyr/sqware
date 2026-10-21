use system_api::identity::CoalitionId;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum LeagueStep {
    Found,
    Admit(CoalitionId),
    Activate(CoalitionId),
    Ready(CoalitionId),
}
