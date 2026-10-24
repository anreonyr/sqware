# Control publisher query checkpoint

This checkpoint adds a generic Control `State` query and native proof using an
ordinary fixture publisher. Production hardware declarations and consumers
remain for the Driver integration; this does not establish A3/A5 completion.

## Wire

- `PublisherAsk` (op 13) carries `namespace: Mark`, `mark: Mark`, `back`.
- It is decoded before the legacy `Ask` name fallback and is bounded by
  `Ask::LEN`; `Req::Publisher` and `Wire::Publisher` carry marks, never a name
  or `TaskId`.
- `Grant::for_wire` maps the query to the existing `State` grant. `Said` and
  all old opcodes are unchanged.

## Query

`Control::publisher(namespace, resource)` scans `self.inputs`, the actual
trusted UnitFile declarations of the deployment start set. A declaration
matches only when:

- its namespace key mark equals the requested namespace mark;
- placement is `Shared`;
- operations contain `BIND`;
- `source_mark == Some(resource mark)` and the resource mark is nonzero.

Any matching declaration authorizes the same program once. Matching
declarations from two distinct program names are `Denied` before the liveness
filter, including when one program is already `Dead`. Exactly one program then
requires its existing table row to be `named`, `Ready`, with a
live nonzero `Slot::Live` task and `Control::live(task)`. Constructed instances
and internal helper tasks are never candidates.

Publisher reports identity, not permission to start work. An accepted explicit
Ruin can take over a recovery job and clear `run_requested` before the external
drain completes. Its live identity remains available during that drain so peers
can authenticate cleanup. `Stopping`, dead tasks and detached slots remain
`NotReady`. Supervisor and Recover admission still require running intent;
consumers must separately enforce admission, target and binding state.

The endpoint returns `Said.task` for a successful query and excludes it from
the lifecycle enqueue path. The client exposes
`Face::publisher(NamespaceKey, Mark, Wait)` and sends only the namespace mark
and resource mark.

## Tests

The focused host crate includes the production `publisher.rs`, `table.rs`,
`verdict.rs`, and `unit/operator.rs`, and covers duplicate same-program
declarations, cross-program ambiguity, unrelated/read-only/instance
declarations, zero marks, missing or non-ready rows, stopping intent during drain, empty
slots, task zero, kernel-dead tasks, a real named ready publisher, and the
bounded op-13 wire roundtrip. The existing control-query endpoint fixture adds
a minimal `publisher` shim solely for dispatch coverage.

Root's final focused Rust checks passed 190 tests across the Publisher, PCI
primitive, existing Control/lifecycle/guard/retirement/shape and image suites.
The 79 Daemon evidence-verifier cases passed. Workspace check passed with five
existing warnings. The first host attempt failed on a test-module include path
and was fixed by importing the unchanged production files through `unit.rs`;
the failed log remains archived.

## Native query and revocation proof

Only the ordinary `fixturetarget` test declaration adds its entry mark as
`source_mark`. The `daemon-stop` controller opens the actual State face and
checks both its kernel owner against the actual System parent and its State
grant mark. Before requesting the kill, it queries Publisher and compares the
returned task to the old target, the actual entry owner and expected entry mark.
After the real Ruin reply and Dead state, every observation must also get
Publisher `NotReady`. The independent verifier requires all seven revocations,
matching task/Root/owner/mark, at least 600 ms observation, namespace absence,
actual whole-Team reaps, normal exit/halt and physical-frame equality. Negative
controls reject foreign tasks/owners/marks and incomplete revocation.

Four/eight-hart stop runs passed in 2.38s/2.57s: actual publisher task 14, Control
host 10, entry owner 14, seven revoked observations, frames 87 -> 87 / 155 -> 155.
Same-source helper recovery passed in 1.92s/1.92s and RTC in 2.47s/2.88s, each
with separately invoked independent verification. Ordinary product 4/8, real
Shell 8, Ruin 8, forced retirement 8 and accept 4 also passed with frame equality.
Evidence is under
`target/driver-composition/device-coordination/publisher-daemon-{stop,target,rtc}-{4,8}/`,
`publisher-regression-*/` and `publisher-root/`.

Independent read-only review found no confirmed source, ABI or security blocker.
The query proves trusted publication intent and current publisher identity;
the consumer must still check the actual endpoint owner and mark. General Peer
authentication, production hardware declarations and dispatch are not yet
integrated by this checkpoint.
