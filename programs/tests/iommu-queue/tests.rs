//! The cases: the completion-cookie policy of the real command queue, driven through
//! the real submission path against the fake device.
//!
//! Witnessed here:
//! - a fence's completion value is bound to *that* submission (a monotone counter, not
//!   the ring index), it lands in its own slot of the completion page, and the command
//!   names the address the device must write to;
//! - an unanswered fence stays in flight: a retry keeps waiting on the same command
//!   (the tail does not move) and a late, older completion cannot pass for it;
//! - the command ring may wrap past its capacity while every cookie stays distinct and
//!   every slot keeps rotating, and a value that owned the current slot a full lap ago
//!   still does not pass;
//! - `disable` reads back quiet CSRs once the device has dropped `ON`.
//! - **the one-step entries.** `raise`/`lower` program their registers and hand back the
//!   wait instead of waiting; `fence_step` is one read (a `Waiting` frame does not run
//!   out the deadline), and every later frame keeps waiting on the *same* submission
//!   under the *same* deadline (`Handshake`), which is what makes the product frame
//!   non-blocking.
//!
//! Not witnessed, and not claimed by these cases:
//! - **`enable`'s success path.** The register window is memory, so `raise` writes
//!   `ENABLE|MEM_FAULT` and reads that same word back inside one call (a store's value
//!   is what the next load sees). A real device raises `ON` on its own, which is not
//!   something a window made of memory can do; the fixture asserts the reading the
//!   driver gets (and the base register it had already programmed), then plays the
//!   device's answer with `Device::rise`.
//! - **The busy handshake.** A read comes back with whatever the window holds, so
//!   `busy` never *rises* here: nothing witnesses "busy rises, then falls".
//! - **Ordering.** `order::sync` is a no-op on the host and the driver and the device
//!   are one thread, so no weak-ordering or device-cache behaviour is exercised.
//! - **Anything below the command ring.** The fake device consumes commands; it does
//!   not translate, walk tables or DMA.
//! - **Real time.** The fake clock steps one millisecond per read, and that is what
//!   ends every bounded wait; the two second deadlines are honoured in that virtual
//!   time, not in wall time.

use alloc::rc::Rc;

use crate::device::{self, COMMANDS, Command, Device, FAULTS, FENCE};
use crate::iommu::queue::{self, CommandQueue, FENCE_DATA};
use crate::iommu::regs::{self, QUEUE_ON};
use crate::iommu::{fence, handshake};

/// Command encoding, as `queue.rs` documents it (its own constants are private):
/// `IOFENCE.C` with `AV` set carries the completion value in `dword0[63:32]`.
const IOFENCE_C: u64 = 2;
const NOTIFY: u64 = 1 << 10;
const PR: u64 = 1 << 12;
const PW: u64 = 1 << 13;
/// One fence waits at most this long (`FENCE_NS` in `queue.rs`).
const FENCE_NS: u64 = 2_000_000_000;
/// Entries in the command ring, and slots in the completion page: `2 << 7`, and the
/// 256 four-byte slots `FENCE_SLOTS` rotates through.
const ENTRIES: u32 = 256;

/// A device whose queues are up, with a queue over it.
fn up() -> (Rc<Device>, CommandQueue) {
    let device = Device::new();
    assert_eq!(
        device.pages().count(),
        4,
        "the window holds the command, fault, completion and register pages"
    );
    let mut queue = CommandQueue::new(device.base(), device.pages(), COMMANDS, FAULTS, FENCE);
    // The one thing this window cannot do is answer a write within the call that made
    // it, so `enable` reads back exactly what it wrote: enabled, memory fault still
    // sticky, no `ON` - a queue that did not come up. The driver says so.
    assert_eq!(queue.enable(), Err("iommu: fault queue did not come up"));
    assert_eq!(
        queue.status(),
        (0, regs::QUEUE_ENABLE | regs::QUEUE_MEM_FAULT)
    );
    // It got as far as programming the fault queue's base; the command queue's base is
    // written only after the handshake passes.
    assert_eq!(queue.base(), device.base());
    assert_eq!(
        queue.bases().1,
        regs::page_number(device.pages().physical(FAULTS))
    );
    assert_eq!(queue.bases().0, 0);
    // The device's answer, played once the driver can no longer be inside the call.
    device.rise();
    assert!(
        queue::active(queue.status().0, QUEUE_ON),
        "the command queue is up"
    );
    assert!(
        queue::active(queue.status().1, QUEUE_ON),
        "the fault queue is up"
    );
    (device, queue)
}

/// The single command the device took off the ring.
fn one_command(device: &Device) -> Command {
    let seen = device.consumed();
    assert_eq!(seen.len(), 1, "exactly one command was rung in");
    seen[0]
}

/// Case 2's starting point: the first fence answered, the second one submitted and in
/// flight ([`Command`] is that second command, `previous` the first one's cookie).
fn outstanding() -> (Rc<Device>, CommandQueue, u32, Command) {
    let (device, mut queue) = up();
    assert!(queue.fence(), "the device answers the first fence");
    let previous = one_command(&device).completion();
    device.hold(true);
    let tail = device.tail();
    assert!(!queue.fence(), "the device holds the second fence");
    assert_eq!(
        device.tail(),
        tail + 1,
        "the second fence was submitted once"
    );
    let in_flight = device.command(tail);
    (device, queue, previous, in_flight)
}

#[test]
fn the_first_fence_lands_its_own_cookie_in_the_first_slot() {
    let (device, mut queue) = up();
    assert!(queue.fence(), "the device answered the fence");
    let command = one_command(&device);
    assert_eq!(command.index, 0, "the first fence is the first command");
    assert_eq!(command.opcode(), IOFENCE_C);
    assert_eq!(
        command.dword0 & NOTIFY,
        NOTIFY,
        "AV: ask for a completion record"
    );
    assert_eq!(
        command.dword0 & PR,
        PR,
        "PR: wait for the reads already issued"
    );
    assert_eq!(
        command.dword0 & PW,
        PW,
        "PW: wait for the writes already issued"
    );
    assert_eq!(
        command.completion(),
        FENCE_DATA,
        "FENCE_DATA + the first cookie"
    );
    assert_eq!(
        command.ring_slot(),
        0,
        "the first command sits in the first ring slot"
    );
    assert_eq!(
        device.slot(command),
        0,
        "its completion sits in the first slot"
    );
    assert_eq!(device.completion_address(command), device.fence_page());
    assert_eq!(
        device.completion(0),
        FENCE_DATA,
        "the value the driver waited for"
    );
    assert_eq!(device.head(), 1, "the device took the command off the ring");
    assert_eq!(queue.heads().0, 1, "CQH says the same");
    assert_eq!(
        device.refused(),
        0,
        "every completion address was in the page"
    );
}

#[test]
fn an_unanswered_fence_stays_in_flight_and_is_not_re_issued() {
    let (device, mut queue) = up();
    assert!(queue.fence(), "the device answers the first fence");
    let first = one_command(&device).completion();
    device.hold(true);
    let tail = device.tail();
    let before = device::now();
    assert!(!queue.fence(), "the device is not answering this fence");
    assert!(
        device::now() - before >= FENCE_NS,
        "the wait ran out its whole deadline before giving up"
    );
    assert_eq!(
        device.tail(),
        tail + 1,
        "the fence was submitted exactly once"
    );
    let in_flight = device.command(tail);
    assert_eq!(
        in_flight.completion(),
        FENCE_DATA + 1,
        "the second fence has its own cookie"
    );
    assert_ne!(
        in_flight.completion(),
        first,
        "which is not the first fence's"
    );
    assert_eq!(device.slot(in_flight), 4, "and it uses the second slot");
    assert!(!queue.fence(), "the retry keeps waiting on the same fence");
    assert_eq!(
        device.tail(),
        tail + 1,
        "a retry must not re-issue the command"
    );
    assert_eq!(
        device.consumed().len(),
        1,
        "the device never saw the second fence"
    );
}

#[test]
fn a_late_store_of_the_previous_cookie_does_not_pass() {
    let (device, mut queue, previous, in_flight) = outstanding();
    let slot = device.slot(in_flight);
    // The first fence's completion turning up late, in the slot this fence watches.
    device.complete(slot, previous);
    assert_eq!(device.completion(slot), previous);
    assert!(
        !queue.fence(),
        "an answer that is not this fence's cannot pass for it"
    );
    assert_eq!(
        device.tail(),
        in_flight.index + 1,
        "and it is still not re-issued"
    );
}

#[test]
fn the_in_flight_fence_passes_once_its_own_cookie_lands() {
    let (device, mut queue, previous, in_flight) = outstanding();
    let slot = device.slot(in_flight);
    device.complete(slot, previous);
    assert!(!queue.fence(), "the older completion is not this fence's");
    device.complete(slot, in_flight.completion());
    assert!(queue.fence(), "the fence's own completion passes it");
    assert_eq!(
        device.tail(),
        in_flight.index + 1,
        "still the one command it submitted"
    );
    assert_eq!(device.consumed().len(), 1);
}

#[test]
fn a_wrapped_ring_keeps_cookies_distinct_and_slots_rotating() {
    let (device, mut queue) = up();
    // Two full laps of both rings: the command ring wraps at 256 entries, and so does
    // the completion page's slot rotation. Every one of these is answered by the device.
    let rounds = 2 * ENTRIES + 8;
    for _ in 0..rounds {
        assert!(queue.fence(), "the running device answers every fence");
    }
    let seen = device.consumed();
    assert_eq!(seen.len() as u32, rounds);
    for (at, command) in seen.iter().enumerate() {
        let at = at as u32;
        assert_eq!(
            command.index, at,
            "commands are consumed in submission order"
        );
        assert_eq!(
            command.completion(),
            FENCE_DATA.wrapping_add(at),
            "one cookie per submission, never a repeat"
        );
        assert_eq!(
            command.ring_slot(),
            (at % ENTRIES) as usize * 16,
            "the command ring rotates"
        );
        assert_eq!(
            device.slot(*command),
            (at % ENTRIES) as usize * 4,
            "the completion slots rotate"
        );
    }
    // The ring really did wrap: command 256 sits in the entry command 0 sat in, and the
    // two carry different cookies.
    assert_eq!(seen[ENTRIES as usize].ring_slot(), seen[0].ring_slot());
    assert_ne!(seen[ENTRIES as usize].completion(), seen[0].completion());
    assert_eq!(device.head(), rounds, "the device consumed all of them");
    assert_eq!(queue.heads().0, rounds, "CQH says the same");
    assert_eq!(device.refused(), 0);

    // Past the wrap, with the device held: the slot in flight is one that a fence a full
    // lap ago also used. Zero it as the driver does, then land that old value in it.
    device.hold(true);
    assert!(!queue.fence());
    let tail = device.tail();
    assert_eq!(tail, rounds + 1, "the in-flight fence was submitted once");
    let in_flight = device.command(rounds);
    let slot = device.slot(in_flight);
    assert_eq!(slot, (rounds % ENTRIES) as usize * 4);
    assert_eq!(
        device.completion(slot),
        0,
        "the driver zeroed the slot before submitting"
    );
    let stale = in_flight.completion().wrapping_sub(ENTRIES);
    let lap_ago = seen[stale.wrapping_sub(FENCE_DATA) as usize];
    assert_eq!(lap_ago.completion(), stale, "a real cookie from a lap ago");
    assert_eq!(device.slot(lap_ago), slot, "and it sat in this very slot");
    device.complete(slot, stale);
    assert!(
        !queue.fence(),
        "the value that owned this slot a lap ago cannot pass"
    );
    device.complete(slot, in_flight.completion());
    assert!(queue.fence(), "this fence's own completion passes it");
    assert_eq!(device.tail(), tail, "neither retry re-issued the command");
}

#[test]
fn a_command_that_asks_for_nothing_does_not_move_the_fence_cookie() {
    let (device, mut queue) = up();
    queue.invalidate_all();
    assert!(queue.fence(), "the device answers the fence");
    let seen = device.consumed();
    assert_eq!(seen.len(), 2, "the invalidation and the fence");
    assert_eq!(seen[0].opcode(), 1, "IOTINVAL.VMA");
    assert_eq!(seen[0].dword0 >> 32, 0, "it carries no completion value");
    assert_eq!(seen[1].opcode(), IOFENCE_C);
    assert_eq!(seen[1].ring_slot(), 16, "the ring index follows the tail");
    assert_eq!(
        seen[1].completion(),
        FENCE_DATA,
        "the first fence is still cookie zero"
    );
    assert_eq!(
        device.slot(seen[1]),
        0,
        "the cookie and its slot count fences, not commands"
    );
    assert_eq!(queue.heads().0, 2);
}

#[test]
fn disable_reports_quiet_csrs_once_the_device_drops_on() {
    let (_device, mut queue) = up();
    assert!(queue::active(queue.status().0, QUEUE_ON));
    assert!(!queue::command_error(queue.status().0));
    // Stopping writes 0: with the queues down, that 0 is what the device shows, and the
    // driver's judgement of "stopped and not busy" has to hold for both queues.
    let (commands, faults) = queue.disable();
    assert!(
        queue::quiet(commands, QUEUE_ON),
        "command queue read back {commands:#x}"
    );
    assert!(
        queue::quiet(faults, QUEUE_ON),
        "fault queue read back {faults:#x}"
    );
    assert_eq!(queue.status(), (0, 0), "and that is what the window holds");
    assert!(!queue::command_error(commands) && !queue::queue_error(faults));
}

#[test]
fn the_fault_queue_is_read_back_as_the_device_wrote_it() {
    let (device, queue) = up();
    assert_eq!(
        queue.faults(),
        (0, 0, 0, 0),
        "no record written, no fault reported"
    );
    // A record's header: cause[11:0] / pid[31:12] / pv[32] / ttype[39:34] / did[63:40].
    let first = 0x2u64 | (0x1u64 << 40);
    let second = 0x21u64 | (0x3Fu64 << 34) | (0x8u64 << 40);
    device.push_fault(1, first);
    assert_eq!(queue.faults(), (1, 0x2, 1, 0), "the record it points at");
    device.push_fault(2, second);
    assert_eq!(queue.faults(), (2, 0x21, 8, 0x3F), "and then the next one");
}

#[test]
fn the_two_error_judgements_share_the_sticky_bit_and_split_on_the_illegal_one() {
    use crate::iommu::regs::{QUEUE_CMD_ILL, QUEUE_CMD_TO, QUEUE_MEM_FAULT, QUEUE_OVERFLOW};
    // Bit 9 is the command queue's timeout and the fault queue's overflow: one bit, two
    // names, so both judgements take it. Bit 10 (illegal command) is the command
    // queue's alone, which is why the two judgements are not one function.
    assert_eq!(QUEUE_CMD_TO, QUEUE_OVERFLOW);
    assert!(queue::command_error(QUEUE_CMD_TO | QUEUE_MEM_FAULT));
    assert!(queue::queue_error(QUEUE_OVERFLOW | QUEUE_MEM_FAULT));
    assert!(queue::command_error(QUEUE_CMD_ILL));
    assert!(!queue::queue_error(QUEUE_CMD_ILL));
    assert!(!queue::command_error(0) && !queue::queue_error(0));
}

#[test]
fn raising_a_queue_programs_its_registers_and_hands_the_wait_back() {
    let device = Device::new();
    let queue = CommandQueue::new(device.base(), device.pages(), COMMANDS, FAULTS, FENCE);
    let before = device::now();
    let handshake = queue.raise(regs::FQCSR);
    // The write happens once: the base register carries the fault page, the control
    // register carries what the protocol says to write, and one read sees both.
    assert_eq!(
        queue.bases().1,
        regs::page_number(device.pages().physical(FAULTS))
    );
    assert_eq!(
        regs::read32(device.base(), regs::FQCSR),
        regs::QUEUE_ENABLE | regs::QUEUE_MEM_FAULT
    );
    // The command queue was not touched: the fault queue is raised first.
    assert_eq!(queue.bases().0, 0);
    assert_eq!(regs::read32(device.base(), regs::CQCSR), 0);
    // And the wait is the caller's: not one read was spent trying to reach the deadline.
    assert!(!handshake.expired(), "raising does not wait");
    assert!(device::now() - before < FENCE_NS);
}

#[test]
fn lowering_a_queue_writes_the_zero_and_hands_the_wait_back() {
    let device = Device::new();
    let queue = CommandQueue::new(device.base(), device.pages(), COMMANDS, FAULTS, FENCE);
    let handshake = queue.lower(regs::CQCSR);
    assert_eq!(regs::read32(device.base(), regs::CQCSR), 0);
    assert!(!handshake.expired(), "lowering does not wait");
    // The fault queue is a separate write with its own value.
    queue.lower(regs::FQCSR);
    assert_eq!(regs::read32(device.base(), regs::FQCSR), 0);
}

#[test]
fn a_single_fence_step_leaves_the_wait_for_the_next_frame() {
    let (device, mut queue) = up();
    assert!(queue.fence(), "the device answers the first fence");
    device.hold(true);
    let tail = device.tail();
    let before = device::now();
    assert!(
        matches!(queue.fence_step(), fence::Fence::Waiting),
        "the held device has not answered"
    );
    assert!(
        device::now() - before < FENCE_NS,
        "one step is one read; it does not run the deadline out"
    );
    assert_eq!(
        device.tail(),
        tail + 1,
        "the step submitted the command once"
    );
    assert!(matches!(queue.fence_step(), fence::Fence::Waiting));
    assert_eq!(device.tail(), tail + 1, "the next frame does not re-issue");
}

#[test]
fn every_frame_waits_on_one_fence_under_one_deadline() {
    let (device, mut queue) = up();
    device.hold(true);
    let before = device::now();
    let mut frames = 0;
    loop {
        match queue.fence_step() {
            fence::Fence::Waiting => {
                frames += 1;
                assert!(frames < 10_000, "the deadline must arrive");
            }
            fence::Fence::Expired => break,
            fence::Fence::Settled => panic!("the device is held"),
        }
    }
    let elapsed = device::now() - before;
    assert!(elapsed >= FENCE_NS, "the wait ran out its whole deadline");
    assert!(
        elapsed < 2 * FENCE_NS,
        "the frames share one deadline, not one each"
    );
    assert_eq!(device.tail(), 1, "submitted once across every frame");
}

#[test]
fn a_handshake_expires_once_after_its_own_limit_however_often_it_is_asked() {
    let handshake = handshake::Handshake::new(FENCE_NS);
    let before = device::now();
    let mut checks = 0;
    while !handshake.expired() {
        checks += 1;
        assert!(checks < 10_000, "the limit must arrive");
    }
    let elapsed = device::now() - before;
    assert!(elapsed >= FENCE_NS, "the limit is honoured");
    assert!(
        elapsed < 2 * FENCE_NS,
        "the limit is fixed at construction, not restarted per check"
    );
}

#[test]
fn the_synchronous_wait_stops_at_the_same_fixed_limit() {
    let handshake = handshake::Handshake::new(FENCE_NS);
    let before = device::now();
    let last = handshake.wait(|| 0u32, |_| false);
    let elapsed = device::now() - before;
    assert_eq!(last, 0, "it hands back the last reading");
    assert!(
        elapsed >= FENCE_NS && elapsed < 2 * FENCE_NS,
        "one fixed limit, not one per read"
    );
}
