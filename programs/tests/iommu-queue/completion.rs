use crate::device::{COMMANDS, Device, FAULTS, FENCE};
use crate::iommu::{cookie, fence::Fence, queue::{CommandQueue, FENCE_DATA, FENCE_SLOTS}};

fn queue(device: &Device) -> CommandQueue {
    CommandQueue::new(device.base(), device.pages(), COMMANDS, FAULTS, FENCE)
}

#[test]
fn unrelated_pending_operations_cannot_consume_each_others_completions() {
    let device = Device::new();
    let mut queue = queue(&device);
    device.hold(true);
    let (mut a_publish, mut a_retire) = queue.tickets(0, 1).unwrap();
    let (mut b_publish, mut b_retire) = queue.tickets(2, 3).unwrap();
    for ticket in [&mut a_publish, &mut a_retire, &mut b_publish, &mut b_retire] {
        assert!(matches!(queue.fence_step(ticket), Fence::Waiting));
    }
    assert_eq!(device.tail(), 4);
    let commands = (0..4).map(|index| device.command(index)).collect::<Vec<_>>();
    device.complete(device.slot(commands[0]), commands[0].completion());
    assert!(matches!(queue.fence_step(&mut a_publish), Fence::Settled));
    for ticket in [&mut a_retire, &mut b_publish, &mut b_retire] {
        assert!(matches!(queue.fence_step(ticket), Fence::Waiting));
    }
    // Even a publish cookie deposited in the retire slot is not retirement proof.
    device.complete(device.slot(commands[1]), commands[0].completion());
    assert!(matches!(queue.fence_step(&mut a_retire), Fence::Waiting));
    device.complete(device.slot(commands[3]), commands[3].completion());
    assert!(matches!(queue.fence_step(&mut b_retire), Fence::Settled));
    assert!(matches!(queue.fence_step(&mut a_retire), Fence::Waiting));
    assert!(matches!(queue.fence_step(&mut b_publish), Fence::Waiting));
    device.complete(device.slot(commands[1]), commands[1].completion());
    assert!(matches!(queue.fence_step(&mut a_retire), Fence::Settled));
    assert!(matches!(queue.fence_step(&mut a_retire), Fence::Settled));
    assert_eq!(device.tail(), 4, "polling completed or pending tickets never resubmits");
}

#[test]
fn a_full_ring_keeps_commands_and_an_unsubmitted_ticket_intact_until_repair() {
    let device = Device::new();
    let mut queue = queue(&device);
    device.hold(true);
    let (mut ticket, _) = queue.tickets(0, 1).unwrap();
    for _ in 0..255 {
        assert!(queue.invalidate_all());
    }
    let first = device.command(0);
    assert!(!queue.invalidate_all());
    device.complete(0, 0x1234);
    assert!(matches!(queue.fence_step(&mut ticket), Fence::Waiting));
    assert_eq!(device.tail(), 255);
    assert_eq!(device.command(0), first, "the oldest command is not overwritten");
    assert_eq!(device.completion(0), 0x1234, "no slot is cleared before submission");
    assert!(!queue.fence(&mut ticket), "waiting for capacity has a fixed deadline");
    assert_eq!(device.tail(), 255);
    device.hold(false);
    device.tick();
    assert!(queue.fence(&mut ticket), "the same ticket resumes after capacity is freed");
    assert_eq!(device.tail(), 256);
    assert_eq!(device.command(255).completion(), FENCE_DATA);
    assert_eq!(device.completion(0), FENCE_DATA);
}

#[test]
fn foreign_queue_tickets_cannot_submit_or_read_a_matching_foreign_completion() {
    let a = Device::new();
    let mut qa = queue(&a);
    let (mut ticket, _) = qa.tickets(0, 1).unwrap();
    let b = Device::new();
    let mut qb = queue(&b);
    b.complete(0, FENCE_DATA);
    assert!(matches!(qb.fence_step(&mut ticket), Fence::Expired));
    assert_eq!(a.tail(), 0);
    assert_eq!(b.tail(), 0);
    assert!(matches!(qa.fence_step(&mut ticket), Fence::Waiting));
    a.tick();
    assert!(matches!(qa.fence_step(&mut ticket), Fence::Settled));
    assert_eq!(a.tail(), 1);
}

#[test]
fn invalid_slots_reserve_no_cookie_and_the_last_slot_stays_within_the_page() {
    let device = Device::new();
    let mut queue = queue(&device);
    for (a, b) in [(0, 0), (0, FENCE_SLOTS), (usize::MAX, 1)] {
        assert!(queue.tickets(a, b).is_none());
    }
    let (mut ticket, _) = queue.tickets(FENCE_SLOTS - 1, 0).unwrap();
    assert!(queue.fence(&mut ticket));
    let command = device.command(0);
    assert_eq!(command.completion(), FENCE_DATA);
    assert_eq!(device.slot(command), 4092);
    assert_eq!(device.refused(), 0);
}

#[test]
fn cookie_exhaustion_is_atomic_nonzero_and_never_wraps() {
    let mut next = Some(u32::MAX - 1);
    assert_eq!(cookie::reserve(&mut next, 3), None);
    assert_eq!(next, Some(u32::MAX - 1));
    assert_eq!(cookie::reserve(&mut next, 2), Some(u32::MAX - 1));
    assert_eq!(next, None);
    assert_eq!(cookie::reserve(&mut next, 1), None);
    let mut next = Some(0);
    assert_eq!(cookie::reserve(&mut next, 1), None);
    assert_eq!(next, Some(0));
    let mut next = Some(1);
    assert_eq!(cookie::reserve(&mut next, 0), None);
    assert_eq!(next, Some(1));
    let mut next = Some(u32::MAX);
    assert_eq!(cookie::reserve(&mut next, 2), None);
    assert_eq!(next, Some(u32::MAX));
    assert_eq!(cookie::reserve(&mut next, 1), Some(u32::MAX));
    assert_eq!(next, None);
}

#[test]
fn waiting_for_capacity_does_not_shorten_the_submitted_fences_deadline() {
    let device = Device::new();
    let mut queue = queue(&device);
    device.hold(true);
    for _ in 0..255 { assert!(queue.invalidate_all()); }
    let (mut ticket, _) = queue.tickets(0, 1).unwrap();
    let start = crate::device::now();
    let release_at = start + 1_900_000_000;
    let completion_at = start + 2_100_000_000;
    let scheduled = device.clone();
    crate::device::on_clock(move |now| {
        let fence_queued = scheduled.tail() > 255;
        scheduled.hold(now < release_at || (fence_queued && now < completion_at));
    });
    assert!(queue.fence(&mut ticket), "the submitted fence gets its own full deadline");
    assert!(crate::device::now() >= completion_at);
    assert!(crate::device::now() < release_at + 2_000_000_000);
    assert_eq!(device.tail(), 256);
    assert_eq!(device.completion(0), FENCE_DATA);
}

#[test]
fn an_unsubmitted_pair_consumes_unique_values_without_touching_hardware() {
    let device = Device::new();
    let mut queue = queue(&device);
    let (_publish, _retire) = queue.tickets(0, 1).unwrap();
    assert_eq!(device.tail(), 0);
    assert_eq!(device.completion(0), 0);
    assert_eq!(device.completion(4), 0);
    let (mut publish, mut retire) = queue.tickets(0, 1).unwrap();
    assert!(queue.fence(&mut publish));
    assert!(queue.fence(&mut retire));
    assert_eq!(device.command(0).completion(), FENCE_DATA + 2);
    assert_eq!(device.command(1).completion(), FENCE_DATA + 3);
    assert_eq!(device.tail(), 2);
}
