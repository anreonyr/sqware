pub use system_api::supply::Binding;

const DESCRIPTOR_SALT: u64 = 0x5355_5050_4c59_4445;

/// Distinct native mailbox marker for the bounded descriptor accompanying a binding.
pub const fn descriptor_mark(slot: env::Mark) -> env::Mark {
    env::Mark::new(slot.get().rotate_left(23) ^ DESCRIPTOR_SALT)
}

/// A unit declares logical bindings only; source definitions belong to providers.
pub fn valid_supplies(bindings: &[Binding]) -> bool {
    bindings.iter().enumerate().all(|(at, binding)| {
        !binding.source.name().is_empty()
            && !binding.resource.name().is_empty()
            && binding.slot != env::Mark::NONE
            && !binding.permission.is_empty()
            && descriptor_mark(binding.slot) != env::Mark::NONE
            && !bindings[..at].iter().any(|old| {
                old.slot == binding.slot
                    || descriptor_mark(old.slot) == descriptor_mark(binding.slot)
            })
    })
}
