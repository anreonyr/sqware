use env::wire::Span as _;
use env::{Mark, Permission, TaskId, Wait};
use resource::raw::{Hole, inspect, pies};

const MARK: Mark = Mark::of("login/system-context");

#[derive(Clone, Copy, env::Frame)]
#[frame(len = 8)]
struct Context {
    system: TaskId,
}

pub(super) fn receive(login: TaskId) -> Result<TaskId, &'static str> {
    let mut found = None;
    for candidate in pies() {
        if let Ok(info) = inspect(candidate.token) {
            if info.alive
                && info.owner == login
                && info.vestor == login
                && info.kind == env::PieKind::Hole
                && info.mark == MARK
                && info.permission.contains(Permission::FETCH)
            {
                if found.is_some() {
                    return Err("shell issuer channel ambiguous");
                }
                found = Some(info.token);
            }
        }
    }
    let token = found.ok_or("shell issuer channel missing")?;
    let mut bytes = [0; Context::LEN];
    let (n, sender) = Hole::from_raw(token)
        .pull(&mut bytes, Wait::AtMost(5000))
        .map_err(|_| "shell issuer channel receive")?;
    if sender != login {
        return Err("shell issuer channel source");
    }
    let (context, end) = Context::fetch_at(&bytes[..n], 0).ok_or("shell issuer frame")?;
    if end != n || context.system.get() == 0 {
        return Err("shell issuer frame");
    }
    Ok(context.system)
}
