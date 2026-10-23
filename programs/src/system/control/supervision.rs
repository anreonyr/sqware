use crate::unit::{Dependency, Target};
use super::unit::start::Input;
use alloc::vec::Vec;

/// Validate the declared supervision group against the deployed start set.
/// The declaration authorizes an existing Deployment target; it never creates one.
pub(crate) fn validate(
    inputs: &[Input],
    targets: &'static [Target],
    guarded: &[&str],
) -> Result<(), &'static str> {
    let mut declared: Option<(&'static crate::unit::UnitFile, &'static str)> = None;
    for input in inputs {
        let program = input.program;
        let Some(group) = program.control.supervision else {
            continue;
        };
        if declared.is_some() {
            return Err("multiple supervision declarations");
        }
        declared = Some((program, group));
    }
    let Some((supervisor, group_name)) = declared else {
        return Ok(());
    };
    if group_name.is_empty() {
        return Err("empty supervision group");
    }
    let target = targets
        .iter()
        .find(|target| target.name == group_name)
        .ok_or("supervision group missing")?;
    if target.members.is_empty() {
        return Err("empty supervision target");
    }
    let ordered = supervisor.control.after.is_some_and(|after| {
        after
            .iter()
            .any(|dependency| matches!(dependency, Dependency::Target(name) if *name == group_name))
    });
    if !ordered {
        return Err("supervision ordering missing");
    }
    let mut seen: Vec<&str> = Vec::new();
    seen.try_reserve_exact(target.members.len())
        .map_err(|_| "supervision capacity")?;
    for member in target.members {
        if *member == supervisor.name() {
            return Err("supervision includes self");
        }
        if guarded.iter().any(|guarded| guarded == member) {
            return Err("supervision includes guarded target");
        }
        if !inputs
            .iter()
            .any(|input| input.program.name() == *member)
        {
            return Err("supervision member not started");
        }
        if seen.iter().any(|seen| seen == member) {
            return Err("duplicate supervision member");
        }
        seen.push(member);
    }
    Ok(())
}
