use alloc::{string::String, vec::Vec};
use env::{PieToken, TaskId, Wait};
use env::wire::Span as _;
use protocol::system::operator::{EntryId, Fail, Permit};
use protocol::system::control::publication::{self as pubcall, Frame, Object, Reply};
use runtime::env::mail::{self, HolePie};
use crate::system::operator::bridge::Tree;
use crate::system::identity::bridge::{Roster, validate, current_authority};
struct Alias {
    name: String,
    object: Object,
    lifetime: Option<TaskId>,
    entry: PieToken,
    pane: EntryId,
    mount: EntryId,
}
pub struct Names { aliases: Vec<Alias> }
impl Names {
    pub fn new() -> Self { Self { aliases: Vec::new() } }
    pub fn entries(&self) -> impl ExactSizeIterator<Item = PieToken> + '_ {
        self.aliases.iter().map(|a| a.entry)
    }
    pub fn register(
        &mut self,
        roster: &Roster,
        tree: &mut Tree,
        name: &str,
        object: Object,
        lifetime: Option<TaskId>,
    ) -> Result<(), &'static str> {
        if !pubcall::valid_name(name) {
            return Err("identity alias name");
        }
        if self
            .aliases
            .iter()
            .any(|a| a.name == name && a.object.kind() == object.kind() && a.object != object)
        {
            return Err("identity alias conflict");
        }
        if self
            .aliases
            .iter()
            .any(|a| a.name == name && a.object == object)
        {
            return Ok(());
        }
        validate(roster, object).map_err(|_| "identity alias source")?;
        self.aliases
            .try_reserve(1)
            .map_err(|_| "identity alias capacity")?;
        let road = object.road(name).ok_or("identity alias path")?;
        let pane = tree.mount(
            road.parent().ok_or("identity alias parent")?,
            None,
            Permit::Public,
            None,
            false,
        )?;
        let entry = match mail::unseal_hole(pubcall::REF) {
            Ok(entry) => entry,
            Err(_) => {
                let _ = tree.unmount(pane);
                return Err("identity alias entry");
            }
        };
        let mount = match tree.mount(
            &road,
            Some(entry),
            Permit::Public,
            Some(runtime::env::unit::self_id()),
            false,
        ) {
            Ok(mount) => mount,
            Err(why) => {
                let _ = mail::seal(entry);
                let _ = mail::release(entry);
                let _ = tree.unmount(pane);
                return Err(why);
            }
        };
        self.aliases.push(Alias {
            name: name.into(),
            object,
            lifetime,
            entry,
            pane,
            mount,
        });
        Ok(())
    }
    fn remove(&mut self, tree: &mut Tree, at: usize) -> Result<(), &'static str> {
        let a = &self.aliases[at];
        tree.unmount(a.mount)?;
        tree.unmount(a.pane)?;
        let _ = mail::seal(a.entry);
        let _ = mail::release(a.entry);
        self.aliases.remove(at);
        Ok(())
    }
    pub fn poll(&mut self, roster: &Roster) {
        let mut bytes = [0; Frame::LEN];
        // Ref answers read only the verified index and never wait for Operator.
        for alias in &self.aliases {
            while let Ok((n, from)) = HolePie::from_token(alias.entry).pull(&mut bytes, Wait::POLL)
            {
                let Some(frame) = Frame::take(&bytes[..n]) else {
                    continue;
                };
                if !valid_back(frame.back, from) {
                    continue;
                }
                let accepted = frame.op == pubcall::RESOLVE
                    && frame.name == alias.name
                    && frame.kind == 10 + alias.object.kind()
                    && Some(alias.object.authority()) == current_authority(roster);
                reply(
                    frame.back,
                    if accepted {
                        Reply::object(alias.object)
                    } else {
                        Reply::fail(Fail::Unjudged)
                    },
                );
            }
        }
    }
    pub fn sweep(&mut self, roster: &Roster, tree: &mut Tree, live: impl Fn(TaskId) -> bool) -> Result<(), &'static str> {
        let authority = current_authority(roster);
        let mut at = 0;
        while at < self.aliases.len() {
            if Some(self.aliases[at].object.authority()) != authority
                || self.aliases[at].lifetime.is_some_and(|task| !live(task))
            { self.remove(tree, at)?; } else { at += 1; }
        }
        Ok(())
    }
}
fn valid_back(back: PieToken, from: TaskId) -> bool {
    matches!(mail::reserve(back), Ok((vestor, owner, mark)) if vestor == from && owner == from && mark == pubcall::BACK)
}
fn reply(back: PieToken, reply: Reply) {
    let mut bytes = [0; Reply::LEN];
    if let Some(n) = reply.store_at(&mut bytes, 0) {
        let _ = HolePie::from_token(back).push(&bytes[..n], Wait::POLL);
    }
    let _ = mail::release(back);
}
