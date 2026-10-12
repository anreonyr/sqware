//! Validate pure UnitFile policy before constructing runtime resources.

pub(crate) fn validate() -> Result<(), &'static str> {
    for program in crate::unit::PROGRAMS {
        if program
            .operator
            .namespace
            .iter()
            .any(|namespace| !namespace.valid())
            || program
                .control
                .construction
                .iter()
                .any(|policy| !policy.valid())
        {
            return Err("invalid UnitFile policy");
        }
    }
    Ok(())
}
