use env::Mark;

const DESCRIPTOR_SALT: u64 = 0x5355_5050_4c59_4445;

/// Distinct native mailbox marker for the bounded descriptor accompanying a binding.
pub const fn descriptor_mark(slot: Mark) -> Mark {
    Mark::new(slot.get().rotate_left(23) ^ DESCRIPTOR_SALT)
}

/// Bounded metadata delivered beside a resource already held by its recipient.
pub const IMAGE_MARK: Mark = Mark::of("unit-image");
