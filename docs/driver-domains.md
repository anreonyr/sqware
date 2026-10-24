# IOMMU domain integration

The production Core owns persistent Domains with independent bindings and IOVA
allocation. Domain and explicit-delivery contracts are recorded below; current
multi-device Driver evidence is in [driver-devices.md](driver-devices.md).
The full refactor and final frozen-source acceptance remain unfinished.

## Command-completion foundation

Each binding reserves separate publish and retire tickets before acquiring its
Frame Reference. A ticket keeps its completion slot, checked nonzero value and
original hardware deadline. Polling, expiry and successful completion do not
resubmit it. A late matching completion can settle the same expired ticket.
Publish completion cannot satisfy retirement.

One CommandQueue remains the only submitter. It retains the cookie issuer across
idle engine transitions. Cookie reservations never wrap. With a 256-entry command
ring, one entry stays empty; a full ring leaves the existing commands and an
unsubmitted ticket intact. Invalidate retries without dropping the reference.
The synchronous helper bounds capacity waiting separately from the submitted
command's deadline.

Frame release rechecks the actual retire value and zeroed device context, in
addition to the existing global queue stop predicate. Published mappings always
require their actual retire completion, including when DDTP is Off. Enabled, busy, active or faulty queues
cannot satisfy this checkpoint's release predicate. The completion pointer names
the stable retained page, not a field in a movable binding record. The owner may
reuse its two slots only after closing its previous binding.

The platform proof is established once before the scope reaches Stopped.
Reference Release failure then retries the exact retained handle. The binding
continues to own its slots and tables, preventing publication or slot reuse until
all remaining release effects finish. Unpublished queue-enable failure skips
IOFENCE while still quiescing any partially raised queues before release.
Startup failure closes its partially acquired, unpublished pool before returning;
an unsettled cleanup is reported and retains the task's remaining kernel handles.
The resident Driver treats startup error as terminal. Its register mapping remains
task-owned until exit; this is not a retryable startup in a long-lived task.

The one-level, single-page DDT holds 64 expanded contexts. Core rejects larger
requesters before taking a data reference or writing the table. Publishing,
invalidation and the release predicate all use the same checked byte offset.

Host coverage compiles the actual Core, Pages, Holds, References, queue and table
implementation. Environment calls and MMIO device responses are fixtures; those
tests do not prove physical DMA, ordering or hardware busy transitions. Native
results are recorded separately from host results and failed invocations.

Remaining integration:

- Persistent IOMMU-owned Domain identity and Domain-qualified address ranges.
- IOVA allocation inside Acquire, independent of BindingId serial, with retries
  retaining the same reservation and no same-Domain address reuse.
- Shared engine/DDT/queues with independent device contexts and completion slots;
  closing one Domain keeps another active Domain working. Ordinary last-mapping
  retirement must reach actual idle Off before final pool reclamation.
- Explicit DMA and BAR capability delivery, retaining both the Coordinator-local
  BAR and its distinct Work-local grant receipt with full resource validation.
- Private device/Domain collections and independently progressing pending RPCs.
- Real two-device/two-Domain equal-IOVA proof and final four/eight-hart regression
  runs, including 1024 generations and the original fault/critical-death gates.

## Ticket-foundation validation checkpoint

Production and DMA test sources were frozen at index tree
`1767fd0eaf80ef2e86b83f8074f85d136e3d0770` for the final native runs.
Afterward, only the structural test's exact module-file list and this result
record changed. The module list now includes `cookie.rs`/`ticket.rs` and excludes
the replaced `outstanding.rs`; its original boundary assertions remain.

Locked host checks passed: Core/Pages 26, command queue 22, retention/Hub ledger
45, Recovery coordination 14, System structure 22, and image library/entry 19:
148 Rust tests total. All 102 Python verifier tests and the workspace check passed.
The workspace still reports five existing warnings.

| Final native run | Harts | Result | Final frame allocations |
| --- | --- | --- | --- |
| Normal DMA | 4 | runner 0 | 85 → 85 |
| Normal DMA | 8 | runner 0 | 153 → 153 |
| Abort before publication | 8 | runner 0 | 153 → 153 |
| Abort after publication | 8 | runner 0 | 153 → 153 |
| Actual CQ fault and Fence repair | 8 | runner/verifier 0 | 153 → 153 |
| Actual Reference Release Busy/retry | 8 | runner/verifier 0 | 155 → 155 |
| In-flight Ruin | 8 | runner/verifier 0 | 153 → 153 |
| In-flight forced retirement | 8 | runner/verifier 0 | 153 → 153 |

Fence repair observed the same real retire value `0x10f00fef`. Reference failure
retained and subsequently released the same native handle 111. Logs are under
`target/driver-composition/domain-composition/native-ticket-final-*/`;
locked host logs use `host-ticket-final-*.log` in the same parent directory.

An earlier Reference scene lacked its required `dma-confirmation-scene` feature
and release debug assertions; its verifier rejected the missing injection. That
failed invocation remains recorded. The corrected run above includes both.
The actual-cookie and early-queue-failure regressions also failed on their prior
production paths before the fixes. No assertion was removed to obtain success.

Independent readonly Flash review accepted the production checkpoint, including
the stricter cookie predicate, retained-handle retry, early-enable close path and
startup rollback. Host and native checks were executed by the root writer, not
independently rerun by Flash. This checkpoint establishes single-binding ticket
ownership; the remaining Domain integration and full final acceptance are open.

## Shared-engine preparation in PR21

Cache barriers now retain their immutable requester and PSCID and independently
admit `IODIR.INVAL_DDT`, scoped `IOTINVAL.VMA`, and their own `IOFENCE.C`.
Queue capacity exhaustion returns Pending and preserves the unsubmitted suffix.
Queue provenance is checked before the first invalidation write. The actual
retire record remains mandatory; global queue quiet is still required until
per-Domain retirement is implemented.

Device contexts encode their 20-bit PSCID in TA bits 31:12. The implementation
currently derives a distinct PSCID from each validated requester; that identifier
is separate from the future opaque DomainId. These cache namespaces and the
separate directory invalidation follow the
[ratified IOMMU specification](https://github.com/riscv-non-isa/riscv-iommu/blob/v1.0/iommu_in_memory_queues.adoc).
Neither DDTP Off nor translation invalidation alone replaces context invalidation.

The retained pool now has four shared pages and three distinct table pages for
each of the 64 contexts: 196 pages. Actual four/eight-hart normal DMA and eight-hart
CQ-fault/Fence-repair runs acquired that pool, completed DMA, and restored frame
allocations 85→85 or 153→153. The hardware verifier reads all three real commands,
checks their requester/PSCID against native identity, and observes the same actual
completion cookie without writing guest completion memory.

Current selected host checks passed: queue 29, Core/Pages 29, structural 22,
and 105 Python verifier cases. Native evidence is in
`target/driver-composition/iommu-domains/native-pool196-*/`. An earlier verifier
invocation rejected a string-to-integer conversion error after the guest runner
succeeded; the corrected verifier rechecked the preserved evidence and the later
pool runs passed the complete runner/verifier path.

That preparation checkpoint still admitted one binding and used the prior wire
and resource-delivery path. The following checkpoint replaces those paths.

## Persistent Domains and explicit delivery in PR21

One IOMMU Core now owns the shared engine and a persistent collection of Domains.
Each Domain belongs to the actual Driver caller and requester, has an opaque
`DomainId { engine, key }`, and retains its own Holds, current binding and retired
serial floor. Every mapping operation carries that Domain and the full Hub binding
identity. A later closure in another Domain cannot reject an earlier pending
binding. The first authenticated binding pins each Domain's Hub.

Acquire validates an explicitly delivered IOMMU-local token and allocates its
range once, before taking the data Reference. Exact retries keep the same range,
completion records and Reference, including after the Work grant is withdrawn.
The private allocator starts at 8 GiB, advances by 2 MiB and stops before the
positive Sv39 limit. Separate Domains can return the same numeric IOVA. The
Domain-qualified range does not encode this private allocation policy. Hub no
longer computes addresses or limits binding serials by an address-space formula.

Each Domain retires through its own DC-zero and actual completion-cookie proof.
Another retained Domain keeps the shared queues running. A Quiesce with no other
engine user lowers the engine. If both Domains become Quiet before either releases,
the last remaining binding's Release lowers and verifies actual Off before releasing
its data Reference. Each earlier release still requires its own zero DC and completed
retire cookie; the
final pool still requires Off, no Busy, quiet queues and no sticky error. A
partially released binding remains responsible for any remaining engine proof.
Unpublished close-before-Acquire also reads back its actual idle engine; it does
not stop another Domain's engine.

Hub forwards only the authenticated device source and an opaque per-binding
client token. Work uses the class client to learn the actual IOMMU target, gifts
the DMA window directly, and reports the returned target-local token. Supply is
authenticated by the actual Work sender and full binding and is immutable on
retry. Window and BAR delivery preserve their target-local grant receipts;
Work no longer scans its resource table for these resources. Activation carries
the actual allocated range to the controller, which also retains the old actual
address for the stale-translation probe.

Final verification used source SHA-256
`856360d2801729f163c95571eaf35881c5577287bcd477d5551bb23433df8f0f`:

- 218 Rust host tests passed: Core/Pages 52, queue 29, retention 45, Recovery
  coordination 15, device recovery/protocol 8, Hub API 4, Hub coordination 10,
  kernel confirmation 6, structural 22, capability import 8, image/entry 19.
- 105 Python verifier tests passed; workspace check passed with five existing
  warnings. Logs are `target/driver-composition/iommu-domains/host-domain-final-*`.
- Native runner and independent verifier both returned 0 for product 4/8 harts,
  and 8-hart before/after-publication abort, hardware Fence repair, actual
  Reference Release Busy, Ruin and forced shutdown. Frame allocations restored
  85→85, 153→153, or 155→155 for the confirmation feature build. Evidence is
  `target/driver-composition/iommu-domains/native-domain-final-*/`.
- The real CQ fault expired and repaired the same `0x10f00fef` completion cookie;
  no completion-memory write was used to manufacture success.
- Readonly Flash review accepted the final ownership and retry contract after
  checking kernel cull semantics and unknown-acknowledgement cleanup. The root
  ran the checks; Flash did not independently rerun native tests.

Failed evidence remains available: an initial invocation enabled release debug
assertions without the scene filter and was terminated as an invalid invocation.
The initial before-publication run completed and restored frames but its verifier
rejected the missing empty-binding release readback. The actual readback was
added, two host regressions protect idle/error and shared-engine behavior, and
all final native cases were rerun on the source above. Assertions were retained.

The Driver still selects one device profile. Its device collection, independent
pending RPCs, two-real-device/equal-IOVA proof and final frozen 4/8-hart plus
1024-generation acceptance remain open. Reset-readback fault injection also
remains unproven; the older reset scene is a transport gate before DeviceStop.
