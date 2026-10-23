#[path = "port_access.rs"]
mod access;
#[path = "port_policy.rs"]
mod policy;
#[path = "port_to.rs"]
mod to;
pub use access::Access;
pub use policy::Policy;
pub use to::To;

pub fn ship(token: crate::PieToken, peer: crate::TaskId, _: Access, _: Policy) -> Result<To, ()> {
    crate::EFFECTS.with(|effects| {
        let mut effects = effects.borrow_mut();
        effects.operations.push((2, token));
        if effects.fail_ship {
            return Err(());
        }
        effects.ships += 1;
        effects.shipped_to.push(peer);
        Ok(To)
    })
}
