# IOMMU domain integration

The first checkpoint separates command completion ownership before introducing
multiple Domains. The full refactor and final frozen-source acceptance remain
unfinished; the production Core still admits one binding.

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

## Shared-engine preparation in PR12

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

Core still admits one binding and still uses the existing wire/resource-delivery
path. Per-Domain collections, address allocation, explicit delivery and real
equal-IOVA two-device composition remain the next integration work.
