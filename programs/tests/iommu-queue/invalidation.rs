use crate::device::{COMMANDS, Device, FAULTS, FENCE};
use crate::iommu::{fence::Fence, invalidation::Invalidation, queue::{CommandQueue, FENCE_DATA}};

#[test]
fn context_eviction_precedes_translation_invalidation_and_the_actual_fence() {
    let device = Device::new();
    let mut queue = CommandQueue::new(device.base(), device.pages(), COMMANDS, FAULTS, FENCE);
    let (ticket, _) = queue.tickets(0, 1).unwrap();
    let mut invalidation = Invalidation::new(8, 9, ticket).unwrap();
    assert!(matches!(invalidation.step(&mut queue), Fence::Settled));
    let commands = device.consumed();
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0].dword0, 3 | (1 << 33) | (8 << 40));
    assert_eq!(commands[0].dword1, 0);
    assert_eq!(commands[1].dword0, 1 | (1 << 32) | (9 << 12));
    assert_eq!(commands[1].dword1, 0);
    assert_eq!(commands[2].opcode(), 2);
    assert_eq!(commands[2].completion(), FENCE_DATA);
    assert!(matches!(invalidation.step(&mut queue), Fence::Settled));
    assert_eq!(device.tail(), 3, "an observed completion does not repeat its prefix");
}

#[test]
fn admission_paused_after_directory_retries_only_the_unsubmitted_suffix() {
    let device = Device::new();
    let mut queue = CommandQueue::new(device.base(), device.pages(), COMMANDS, FAULTS, FENCE);
    let (ticket, _) = queue.tickets(0, 1).unwrap();
    let mut invalidation = Invalidation::new(63, 64, ticket).unwrap();
    device.hold(true);
    for _ in 0..254 { assert!(queue.invalidate_all()); }
    assert!(matches!(invalidation.step(&mut queue), Fence::Waiting));
    assert_eq!(device.tail(), 255);
    assert_eq!(device.command(254).dword0, 3 | (1 << 33) | (63 << 40));
    assert!(matches!(invalidation.step(&mut queue), Fence::Waiting));
    assert_eq!(device.tail(), 255);
    device.hold(false);
    device.tick();
    assert!(matches!(invalidation.step(&mut queue), Fence::Settled));
    assert_eq!(device.tail(), 257);
    assert_eq!(device.command(255).dword0, 1 | (1 << 32) | (64 << 12));
    assert_eq!(device.command(256).opcode(), 2);
    assert_eq!(device.command(256).completion(), FENCE_DATA);
    assert_eq!(device.consumed().iter().filter(|command| command.opcode() == 3).count(), 1);
}

#[test]
fn admission_paused_before_fence_preserves_both_invalidations_and_its_ticket() {
    let device = Device::new();
    let mut queue = CommandQueue::new(device.base(), device.pages(), COMMANDS, FAULTS, FENCE);
    let (ticket, _) = queue.tickets(0, 1).unwrap();
    let mut invalidation = Invalidation::new(8, 9, ticket).unwrap();
    device.hold(true);
    for _ in 0..253 { assert!(queue.invalidate_all()); }
    assert!(matches!(invalidation.step(&mut queue), Fence::Waiting));
    assert_eq!(device.tail(), 255);
    assert_eq!(device.command(253).opcode(), 3);
    assert_eq!(device.command(254).opcode(), 1);
    assert!(matches!(invalidation.step(&mut queue), Fence::Waiting));
    assert_eq!(device.tail(), 255);
    device.hold(false);
    device.tick();
    assert!(matches!(invalidation.step(&mut queue), Fence::Settled));
    assert_eq!(device.tail(), 256);
    assert_eq!(device.command(255).completion(), FENCE_DATA);
    assert_eq!(device.consumed().iter().filter(|command| command.opcode() == 3).count(), 1);
}

#[test]
fn foreign_queue_rejects_an_invalidation_before_any_directory_or_translation_command() {
    let a = Device::new();
    let mut qa = CommandQueue::new(a.base(), a.pages(), COMMANDS, FAULTS, FENCE);
    let (ticket, _) = qa.tickets(0, 1).unwrap();
    let mut invalidation = Invalidation::new(8, 9, ticket).unwrap();
    let b = Device::new();
    let mut qb = CommandQueue::new(b.base(), b.pages(), COMMANDS, FAULTS, FENCE);
    assert!(matches!(invalidation.step(&mut qb), Fence::Expired));
    assert_eq!(b.tail(), 0);
    assert_eq!(a.tail(), 0);
    assert!(matches!(invalidation.step(&mut qa), Fence::Waiting));
    a.tick();
    assert!(matches!(invalidation.step(&mut qa), Fence::Settled));
    assert_eq!(a.tail(), 3);
}

#[test]
fn invalid_subjects_never_submit_a_command() {
    let device = Device::new();
    let mut queue = CommandQueue::new(device.base(), device.pages(), COMMANDS, FAULTS, FENCE);
    let (ticket, _) = queue.tickets(0, 1).unwrap();
    assert!(Invalidation::new(64, 1, ticket).is_none());
    let (ticket, _) = queue.tickets(0, 1).unwrap();
    assert!(Invalidation::new(8, crate::iommu::context::PSCID_LIMIT, ticket).is_none());
    assert!(!queue.invalidate_translations(crate::iommu::context::PSCID_LIMIT));
    assert!(!queue.invalidate_context(64));
    assert_eq!(device.tail(), 0);
}

#[test]
fn a_full_first_step_has_no_effect_and_one_free_entry_admits_only_its_directory_command() {
    let device = Device::new();
    let mut queue = CommandQueue::new(device.base(), device.pages(), COMMANDS, FAULTS, FENCE);
    let (ticket, _) = queue.tickets(0, 1).unwrap();
    let mut invalidation = Invalidation::new(8, 9, ticket).unwrap();
    device.hold(true);
    for _ in 0..255 { assert!(queue.invalidate_all()); }
    assert!(matches!(invalidation.step(&mut queue), Fence::Waiting));
    assert_eq!(device.tail(), 255);
    assert_eq!(device.command(254).dword0, 1);
    device.hold(false);
    device.consume(1);
    device.hold(true);
    assert!(matches!(invalidation.step(&mut queue), Fence::Waiting));
    assert_eq!(device.tail(), 256);
    assert_eq!(device.command(255).dword0, 3 | (1 << 33) | (8 << 40));
    assert!(matches!(invalidation.step(&mut queue), Fence::Waiting));
    assert_eq!(device.tail(), 256);
    device.hold(false);
    device.tick();
    assert!(matches!(invalidation.step(&mut queue), Fence::Settled));
    assert_eq!(device.tail(), 258);
    assert_eq!(device.command(256).dword0, 1 | (1 << 32) | (9 << 12));
    assert_eq!(device.command(257).completion(), FENCE_DATA);
    assert_eq!(device.consumed().iter().filter(|command| command.opcode() == 3).count(), 1);
}

#[test]
fn separate_requesters_and_address_spaces_keep_their_own_admission_and_completion() {
    let device = Device::new();
    let mut queue = CommandQueue::new(device.base(), device.pages(), COMMANDS, FAULTS, FENCE);
    let (a, _) = queue.tickets(0, 1).unwrap();
    let (b, _) = queue.tickets(2, 3).unwrap();
    let mut a = Invalidation::new(8, 9, a).unwrap();
    let mut b = Invalidation::new(16, 17, b).unwrap();
    device.hold(true);
    assert!(matches!(a.step(&mut queue), Fence::Waiting));
    assert!(matches!(b.step(&mut queue), Fence::Waiting));
    assert_eq!(device.tail(), 6);
    assert_eq!(device.command(0).dword0, 3 | (1 << 33) | (8 << 40));
    assert_eq!(device.command(1).dword0, 1 | (1 << 32) | (9 << 12));
    assert_eq!(device.command(3).dword0, 3 | (1 << 33) | (16 << 40));
    assert_eq!(device.command(4).dword0, 1 | (1 << 32) | (17 << 12));
    let b_fence = device.command(5);
    device.complete(device.slot(b_fence), b_fence.completion());
    assert!(matches!(b.step(&mut queue), Fence::Settled));
    assert!(matches!(a.step(&mut queue), Fence::Waiting));
    assert_eq!(device.tail(), 6);
}
