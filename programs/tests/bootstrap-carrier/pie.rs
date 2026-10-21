pub fn narrow(token: crate::PieToken, permission: crate::Permission) -> Result<(), ()> {
    assert_eq!(permission, crate::Permission::STORE);
    crate::EFFECTS.with(|effects| {
        let mut effects = effects.borrow_mut();
        effects.operations.push((3, token));
        if effects.fail_narrow {
            return Err(());
        }
        effects.narrows += 1;
        Ok(())
    })
}

pub fn release(token: crate::PieToken, mode: crate::ReleaseMode) -> Result<(), ()> {
    assert_eq!(mode, crate::ReleaseMode::Revoke);
    crate::EFFECTS.with(|effects| {
        let mut effects = effects.borrow_mut();
        effects.operations.push((4, token));
        if effects.fail_release {
            return Err(());
        }
        effects.releases += 1;
        Ok(())
    })
}
