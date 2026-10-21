use super::*;

pub(super) fn block_with_call<E: WaitFail>(
    key: WakeKey,
    life: Weak<Life>,
    dur: Duration,
    wait: Option<WaitCall>,
) -> Result<Blocked, E> {
    if take_beacon(key) {
        return Ok(Blocked::Resume);
    }
    let Some(me) = current().running_task() else {
        return Err(E::busy());
    };
    let _commit = crate::core::work::commit();
    if let Some(reason) = super::super::take_doomed(me.ident.id) {
        drop(me);
        return Ok(Blocked::Quit(reason));
    }
    {
        let mut sites = sites(key).lock();
        Site::ensure(&mut sites, key, &life).map_err(|()| E::oom())?;
    }
    let ticket = Ticket::next_id();
    let at = (dur != Duration::MAX).then(|| clock::now().add(dur).as_ticks());
    if let Some(at) = at {
        hold(ticket, key, &me).map_err(|()| E::oom())?;
        if deadline::tock(ticket.raw(), at).is_err() {
            void(ticket);
            return Err(E::oom());
        }
    }
    drop(me);
    let (task, next_pa) = current().swap();
    trace::note(EventKind::Room(RoomEvent::Wait {
        tid: task.ident.id.get(),
        key: key.fold() as usize,
    }));
    task.block(key, ticket, wait);
    let queued = {
        let mut sites = sites(key).lock();
        let queued = match sites.get_mut(&key) {
            None => false,
            Some(site) if Life::dead(&site.life) => false,
            Some(site) if site.pend => {
                site.pend = false;
                false
            }
            Some(site) => {
                site.push_back(task.clone());
                true
            }
        };
        prune(&mut sites, key);
        queued
    };

    if queued {
        drop(task);
    } else {
        void(ticket);
        rise(core::iter::once(task));
    }
    #[cfg(debug_assertions)]
    crate::core::work::unit::copies::check_block_heldout();
    Ok(Blocked::Switch(next_pa))
}
