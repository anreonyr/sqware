//! Validate pure UnitFile policy before constructing runtime resources.

pub(crate) fn validate() -> Result<(), &'static str> {
    for (program_index, program) in crate::unit::PROGRAMS.iter().enumerate() {
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
        for (namespace_index, namespace) in program.operator.namespace.iter().enumerate() {
            for earlier in crate::unit::PROGRAMS[..=program_index].iter() {
                let limit = if core::ptr::eq(earlier, program) {
                    namespace_index
                } else {
                    earlier.operator.namespace.len()
                };
                for previous in earlier.operator.namespace[..limit].iter() {
                    if previous.key.mark() == namespace.key.mark()
                        && previous.key.name() != namespace.key.name()
                    {
                        return Err("namespace key collision");
                    }
                }
            }
        }
    }
    Ok(())
}
