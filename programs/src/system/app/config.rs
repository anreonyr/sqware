//! Validate pure UnitFile policy before constructing runtime resources.

pub(crate) fn validate(declarations: &[&unit::UnitFile]) -> Result<(), &'static str> {
    for (program_index, program) in declarations.iter().enumerate() {
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
            || program.control.retirement.iter().any(|retirement| {
                !retirement.valid()
                    || !declarations.iter().any(|provider| {
                        provider.name() == retirement.provider
                            && retirement.namespace < provider.operator.namespace.len()
                    })
            })
        {
            return Err("invalid UnitFile policy");
        }
        for (namespace_index, namespace) in program.operator.namespace.iter().enumerate() {
            for earlier in declarations[..=program_index].iter() {
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
