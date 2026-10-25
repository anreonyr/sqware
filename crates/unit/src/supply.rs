use crate::Binding;

/// A unit declares logical bindings only; source definitions belong to providers.
pub fn valid_supplies(bindings: &[Binding]) -> bool {
    bindings.iter().enumerate().all(|(at, binding)| {
        !binding.source.name().is_empty()
            && !binding.resource.name().is_empty()
            && binding.slot != env::Mark::NONE
            && !binding.permission.is_empty()
            && system_api::supply::descriptor_mark(binding.slot) != env::Mark::NONE
            && !bindings[..at].iter().any(|old| {
                old.slot == binding.slot
                    || system_api::supply::descriptor_mark(old.slot)
                        == system_api::supply::descriptor_mark(binding.slot)
            })
    })
}
