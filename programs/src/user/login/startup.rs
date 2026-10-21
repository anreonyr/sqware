use env::wire::Span as _;
use env::{Mark, Permission, PieToken, TaskId, Wait};
use resource::raw::{Hole, inspect};

const MARK: Mark = Mark::of("login/system-context");

#[derive(Clone, Copy, env::Frame)]
#[frame(len = 8)]
struct Context {
    system: TaskId,
}

pub(crate) struct Issuer(PieToken);

impl Drop for Issuer {
    fn drop(&mut self) {
        let _ = env::pie::release(self.0, env::ReleaseMode::Revoke);
    }
}

pub(crate) fn deliver(child: TaskId, system: TaskId) -> Result<Issuer, ()> {
    let token = env::hole::unseal(MARK, env::HoleLimits::default()).map_err(|_| ())?;
    let issuer = Issuer(token);
    let info = inspect(token).map_err(|_| ())?;
    let login = env::unit::self_id();
    if !info.alive
        || info.kind != env::PieKind::Hole
        || info.owner != login
        || info.vestor != TaskId::new(0)
        || !info
            .permission
            .contains(Permission::STORE | Permission::FETCH)
    {
        return Err(());
    }
    let mut frame = [0; Context::LEN];
    let n = Context { system }.store_at(&mut frame, 0).ok_or(())?;
    if Hole::from_raw(token)
        .push(&frame[..n], Wait::AtMost(1000))
        .is_err()
    {
        return Err(());
    }
    if env::pie::accord(token, child, Permission::FETCH, MARK).is_err() {
        return Err(());
    }
    Ok(issuer)
}
