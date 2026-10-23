# Driver coordination checkpoint

The coordination face carries a Hub-qualified binding identity and execution
task to an independently scheduled driver. It does not authorize its sender or
prove CPU retirement. Concrete hardware calls remain in Hub at this checkpoint;
the A3/A5 migration is still required.

`hub_api::coordination::Prepare` carries `{hub, serial, key, execution, back}`;
`Release` carries `{hub, serial, execution, reason, back}`. Both use the existing
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
