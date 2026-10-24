pub fn unseal(_: crate::Mark, _: crate::HoleLimits) -> Result<crate::PieToken, ()> {
    crate::EFFECTS.with(|effects| {
        let mut effects = effects.borrow_mut();
        if effects.fail_unseal {
            return Err(());
        }
        effects.unseals += 1;
        Ok(crate::PieToken::mint(effects.unseals))
    })
}
