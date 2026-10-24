# Driver coordination checkpoint

The coordination face carries a Hub-qualified binding identity and execution
task to an independently scheduled driver. It does not authorize its sender or
prove CPU retirement. The current integration moves concrete hardware calls into
the resident device Driver; Hub retains logical claims and binding responsibility.
The complete refactor and its final frozen-source validation remain unfinished.

`hub_api::coordination::Prepare` carries `{hub, serial, key, execution, back}`;
`Release` carries `{hub, serial, key, execution, reason, back}`. Both use the existing
`DmaSaid` response. Frames reject truncated/trailing data, wrong operations and
zero Hub, serial or execution identities. Device keys are opaque: zero is valid.
The runtime must authenticate the actual kernel sender and stored binding before
effects; the wire identity alone is only a coordinate.

`Peer::open_checked` borrows an entry and checks its kernel owner and mark against
an independently authenticated expected identity. `Peer::find_checked` rejects
invalid expectations before lookup. If a fresh lookup delivers a reference but
identity validation fails, the error returns that reference to the caller, which
must release it or retain it for cleanup. It never silently discards the handle.
`Control::Face::of_owned` checks the expected System identity supplied by trusted
composition, such as the actual parent of a static unit.

Publisher reports a trusted live identity during external cleanup even after an
accepted Ruin takeover clears running intent. It is not new-work authority.
The production takeover, intent, Publisher and authority code is exercised in
the same host test: both Recover and Embark takeovers retain the old identity
while effect recheck denies further work. The old Publisher predicate fails
the new drain-window regression; its failure log is retained.

Host checks passed: coordination 9, Publisher 11, daemon-control 14, and 80
Daemon verifier cases. Workspace check passed with five existing warnings.
The initial native invocation omitted the `scene` test filter, started unrelated
kernel tests, and was stopped after failures; it is not a passing result.

Correctly filtered four/eight-hart `daemon-stop` runs passed, with independent
verification. Both rejected seven identity/lookup cases and explicitly reclaimed
two newly delivered references without damaging the original State entry.
Publisher revocation still covered all seven post-Ruin observations. Actual
whole-Team reaps and final frame equality remained mandatory: 87 to 87 and
155 to 155. Evidence is in
`target/driver-composition/device-coordination/coord-identity-scene-stop-{4,8}/`;
host logs are in `coord-api-root/`.

On the same source, product 4/8, Shell 8, accept 4 and Ruin/forced retirement 8 passed with
raw runner exit zero and final frames 87/155 restored. Existing Control query,
RPC, API, structure and Hub handoff host suites passed. These checks establish
this interface/identity checkpoint, not the remaining hardware migration or
the final multi-device/multi-domain integration requirement.

Independent Flash review found no confirmed source security/ownership defect.
It identified a coverage gap: the takeover test exercised both actions but
rechecked only Embark. It now rechecks the matching Recover or Embark action.
Queued return obligations alone do not prove CPU retirement. The actual-target
receipt is described below; hardware migration remains subsequent work.

## Actual retirement target

The bounded Return adapter stores `target_received` independently from receipt
ownership. Only the trusted Control per-target ASK path sets it. Closing,
expiry, global settlement, owner inspection and receipt trust do not set it.
Target queue admission precedes admission closure and hardware advancement. A
full queue answers Pending and requires Control to retry the same target.
NoReceipt cannot discard this fact while the task has an open binding.

Return still sends the early Control Release request: this induces CPU stop
and the per-target retirement callback. Waiting for hardware closure before
sending that request would deadlock. Successful release remains gated by the
existing real Control retirement path.

The production handoff host suite passed 14 tests, including distinct target
proof, failed queue admission and explicit retry, NoReceipt retention with an
open binding, and the existing early Release request/order/error checks.
Product 4/8, Ruin/forced retirement 8 and accept 4 passed with raw exit zero and
frame equality. This still uses the existing hardware chain; the Driver
runtime and Hub asynchronous adapter must be integrated next.

Independent review rechecked the full Control path and withdrew two earlier
findings: an early Release request does not imply an early successful ACK,
and global settlement reaches actual per-target callbacks. Retirement hooks
start after whole-Team completion; declared retirement dependencies must finish
before the hook marks the instance Dead. Only then can release succeed.
The review found no confirmed source defect in this target-receipt change.
Additional Control instance (33), lifetime (8), retirement-boundary (11) and
shutdown-policy (8) host tests passed. No duplicate receipt state or global CPU
permit was introduced.

## Current integration

PCI publishes bounded read-only configuration snapshots and explicit requester/BAR
effects. Device selection and Virtio capability parsing run in the device Driver.
Hub no longer imports concrete PCI/IOMMU/Recovery calls or a hardware-step cursor.
Each binding retains an independent pending coordination RPC. Hardware Release
requires the existing trusted Control target receipt.

The Driver publishes its coordination endpoint before peer discovery. Every
imported provider is checked against a declaration-derived Publisher identity,
actual kernel owner and mark. Failed imports remain retained for reclamation.
Hub DMA and PCI snapshot publication have separate scoped namespace grants.

The device Driver retains partial acquisition handles and the first observed
failure. Each Release request performs one cleanup effect and retains the next
stage for the following request. The hardware order remains Invalidate, Disable,
device Stop, Quiesce, IOMMU Release, Detach, handoff reclamation, PCI Release.

The former reset confirmation scene rejected transport before device Stop. Its
equivalent cut now rejects the next Hub-to-Driver Release after actual PCI
Disable. The kernel fixture authenticates both task pairs: the gate target is
also the Disable sender. Its actual 21-byte PCI frame arms the gate; the 42-byte
coordination frame is then rejected. Native evidence requires MasterOff before
rejection, two retained IOMMU references, and device Stop only after removal.
The first failure belongs to Hub because the Driver never received the request.
This is transport-failure evidence; hardware reset-readback rejection is unproved.

Current host checks include 14 Coordinator, 8 PCI placement, 5 recovery, 6 kernel
confirmation and 102 Python verifier cases. Native eight-hart evidence includes
normal/fault DMA, publication aborts, in-flight Ruin/forced retirement, real Fence
fault/repair and Reference Release failure/retry. The migrated Stop transport gate
reclaimed frames 155 to 155. Three guarded Driver deaths independently passed the
halt verifier with raw runner timeout 101 and seven peer-hart quiescence receipts.
These runs are incremental evidence, not the final frozen-source acceptance.

DeviceId now contains the authenticated source and an opaque u64 key. Enrollment
does not accept a self-reported source. Pending publication, device lookup,
claim retries and occupied-binding counts use the complete identity. Tests
register two sources with the same key and distinct doors, preserve failed
publication handles, and prove that completing one binding leaves the other
occupied. The adapter refuses to send a matching key to a different Driver.
The current deployment still has one authenticated coordination provider;
multiple-provider endpoint collections remain part of the unfinished extension.

The wire codec preserves zero and high key bits and rejects truncated/trailing
frames. Current identity checks pass 45 ledger, 21 Hub adapter, 10 coordination
codec and 22 structural cases. Actual directory claims completed four-hart DMA
and eight-hart in-flight Ruin, with frames 85 to 85 and 153 to 153.

Domain/range identity, IOMMU-owned IOVA allocation,
explicit capability delivery, private device/domain collections,
two-device/two-domain equal-IOVA proof and final 1024 generations remain open.
The generic ECAM probe now validates Type 0/1 BAR counts, 64-bit lower-slot
boundaries and supported memory types before writing. Actual-Ecam RAM tests cover
slot classification and restoration. Native eight-hart DMA exercised an actual
reset-zero 32-bit BAR (4 KiB, restored to zero, MASTER off) and rejected the upper
half of the placed 64-bit BAR before completing real DMA and stale-DMA isolation.
The RAM checks do not emulate hardware size masks. General placement of 32-bit
devices still requires a suitable granted low-address aperture.

## PR10 validation checkpoint

Production and test sources were frozen at index tree
`8ab59c5dbd1ff2b410f8f54e6e8df3dbdd2568fd`; the following updates only record
results. The branch stacks on `codex/daemon-supervision` (PR9).

- Rust: 175 passing cases across ledger 45, Hub adapter 21, wire 10,
  Coordinator 14, PCI placement 8, PCI scan/ECAM 21, recovery 5, Virtio parser 7,
  structural boundaries 22, kernel confirmation 6 and image 16.
- Python evidence checkers: 102 cases passed, including wrong source, premature
  release, stale trigger, missing native completion and incorrect reclamation.
- Native: four-hart DMA product; eight-hart fault/rebind, both publication
  aborts, in-flight Ruin, forced retirement and real Fence fault/repair passed.
  Ordinary frames returned to 85/85 or 153/153.
- All four confirmation scenes passed on eight harts: invalidation RPC,
  transport before device Stop, IOMMU release RPC and actual Reference Release
  failure. Two IOMMU references stayed owned during failure; retry used the same
  binding/reference handle. Final frames were 155/155.
- PCI, IOMMU and device Driver in-flight deaths each passed on four and eight
  harts. The raw runner code was the expected 101; the independent verifier
  accepted retained NBD state, no cleanup and exactly three/seven quiesced peers.
- Real reset-zero 32-bit BAR sizing/restoration and 64-bit upper-half rejection
  passed on four/eight harts before the DMA probe's positive and negative checks.
- Ordinary product on eight harts, accept on four and IOMMU probe on eight
  harts passed. Workspace check, locked host runs and diff checks passed.

The authorized independent Flash group finished its namer, worker and reviewer
turns before handoff. Scoped reviews covered Hub's asynchronous adapter,
partial acquisition ownership, boot authentication, bounded cleanup/transport
gate, actual ECAM slot walking and source-qualified identities. No confirmed
source blocker remains in this checkpoint. A8-A12 completion, multi-device
runtime proof and final integrated 1024-generation acceptance remain open.
