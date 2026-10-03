pub(crate) fn device(
    from: env::TaskId,
    target: &protocol::system::control::publication::Target,
    mark: env::Mark,
    requested: protocol::system::operator::Permit,
    machine: &crate::system::common::machine::Machine,
    roster: &crate::system::identity::serve::install::Roster,
) -> Result<protocol::common::path::PathBuf, protocol::system::operator::Fail> {
    use crate::system::identity::serve::query::{binding, validate};
    use protocol::common::path::Path;
    use protocol::system::control::publication::{Object, Scope, Target};
    use protocol::system::identity::Selector;
    use protocol::system::operator::{Fail, Permit};
    match target {
        Target::Service {
            scope: Scope::Device,
            group,
            name,
        } => {
            if mark != protocol::service::hub::Grant::Claim.mark() {
                return Err(Fail::Denied);
            }
            let Permit::Identity(Selector::MemberOf(c)) = requested else {
                return Err(Fail::Denied);
            };
            let subject = binding(roster, from)?.ok_or(Fail::Denied)?.current;
            if !subject.coalitions.contains(c) {
                return Err(Fail::Denied);
            }
            let valid = (group == protocol::service::hub::BOOT
                && [protocol::service::hub::DTB, protocol::service::hub::IRQ]
                    .contains(&name.as_str()))
                || machine.devices().is_some_and(|devices| {
                    devices
                        .iter()
                        .any(|d| d.class.as_str() == group && d.name.as_str() == name)
                });
            if !valid {
                return Err(Fail::Denied);
            }
            validate(roster, Object::Coalition(c))?;
            Path::new("dev")
                .try_join(group)
                .and_then(|p| p.try_join(name))
                .ok_or(Fail::Denied)
        }
        _ => Err(Fail::Denied),
    }
}
