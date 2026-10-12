use env::TaskId;
use system_api::identity::export::Request;
use system_api::identity::{IdentityObject, Subject};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Bad,
    Denied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExistingGrant {
    Delivered,
    Retry,
    Conflict,
}

pub(crate) fn existing_grant(same_program: bool, delivered: bool) -> ExistingGrant {
    if !same_program {
        ExistingGrant::Conflict
    } else if delivered {
        ExistingGrant::Delivered
    } else {
        ExistingGrant::Retry
    }
}

pub(crate) struct CallerFacts {
    pub from: TaskId,
    pub owner: TaskId,
    pub live: bool,
    pub named: bool,
}

pub(crate) struct AliasClaim<'a> {
    pub aliases: &'a [&'a str],
    pub request: &'a Request,
}

pub(crate) fn authorize_alias(
    caller: CallerFacts,
    claim: AliasClaim<'_>,
    mut allow_subject: impl FnMut(TaskId, Subject) -> Result<(), ()>,
) -> Result<Subject, Failure> {
    let CallerFacts {
        from,
        owner,
        live,
        named,
    } = caller;
    let AliasClaim { aliases, request } = claim;
    if from != owner || !live || !named || !aliases.contains(&request.alias.as_str()) {
        return Err(Failure::Denied);
    }
    let IdentityObject::Principal(principal) = request.object else {
        return Err(Failure::Denied);
    };
    let subject = Subject::new(principal, &[]).map_err(|_| Failure::Bad)?;
    allow_subject(from, subject).map_err(|_| Failure::Denied)?;
    Ok(subject)
}

pub(crate) fn keep_lookup_resources(
    located: env::PieToken,
    actual: Option<env::PieToken>,
    mut release_keep: impl FnMut(env::PieToken) -> Result<(), ()>,
) -> Result<(), ()> {
    let mut result = Ok(());
    if actual == Some(located) {
        if release_keep(located).is_err() {
            result = Err(());
        }
        return result;
    }
    if release_keep(located).is_err() {
        result = Err(());
    }
    if let Some(actual) = actual {
        if release_keep(actual).is_err() {
            result = Err(());
        }
    }
    result
}
