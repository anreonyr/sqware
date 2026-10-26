# Stage 3 IPC extraction

The IPC mechanism no longer lives in this tree. Its provider is
<https://github.com/shuyra-arche/ipc.git> at full revision
`29ad97dc08d9dfc4ea6578b1b77874261229b3b5`. The provider maps the 32 tracked
files from the former `crates/ipc` to the same relative paths; the 16
production `.rs` files are byte-identical. Per-file hashes and the local
provider history are in the provider's `PROVENANCE.md`. The old
`crates/ipc` directory, including its two ignored owner locks, was removed; no
other crate was deleted and no second copy of the extracted `crates/ipc`
implementation remains in source.

## Evidence location

Consumer evidence is outside this source tree and is not tracked here. Its
exact root is `/home/anreonyr/Develop/shuyra/evidence/pr15/consumer`; from this
source checkout that is `../evidence/pr15/consumer`. References of the form
`evidence/pr15/consumer/...` in this document are relative to
`/home/anreonyr/Develop/shuyra` (the parent of this source tree), not to the
source root.

## Consumers

Fourteen manifests now use the canonical Git provider, with fifteen
dependency occurrences:

- `programs/Cargo.toml` and the six clients under
  `programs/src/{driver/router,service/account,service/hub,service/pipe,system,user/terminal}/client`
  keep `ipc` as a normal dependency.
- The seven test manifests under
  `programs/tests/{namespace-layout,operator-client,operator-connection,startup-authority,supply,terminal-connection,virtio-blk-coord}`
  keep their existing aliases/tables; `virtio-blk-coord` keeps `ipc_impl` in
  `[dev-dependencies]`, and the other tests keep it normal.
- `operator-client` adds the dev-dependency
  `ipc_exchange_fixture = { package = "ipc-session-exchange-host", git = …, rev = … }`.
  The fixture has no dependencies.

`"crates/ipc"` was removed from the root workspace `members`.

## Locks

Exactly eight locks changed: the root `Cargo.lock` and the locks of the seven
consumer test packages above. Their full package records were compared against
the frozen `7848438d…` `git show` baseline. The only allowed changes are the
`ipc` package source moving from a path to the canonical Git revision, and, in
`operator-client`, the new fixture package plus its one dependency edge. No
checksum, version or other package field drifted. The remaining 52 locks are
not 52 tracked locks and not 52 locks compared by `git show`: 44 are tracked
source locks compared by full package records against the frozen `7848438d…`
`git show` baseline, and 8 are pre-existing ignored local locks compared
against hashes recorded before validation. All 52 are byte-identical.

The provider's freshly generated root lock resolved `unicode-ident 1.0.27` and
`bitflags 2.13.2`; the source root lock retains `1.0.24` and `2.13.1`. This is a
fresh resolution for the smaller provider workspace, not a consumer upgrade.
The three retained provider owner locks keep their original versions (`host`
`unicode-ident 1.0.24`, `rack-reader-host` `1.0.26`). Existing full Git pins,
checksums and the unrelated alloc-probe vendor patch were not changed.

## Test ownership

The two lookup regression tests moved from the provider fixture to
`programs/tests/operator-client/lookup_regression.rs`: one file, one
`ContractNum` data structure, and the same assertions. It projects the same
owner module
(`#[path = "../../src/system/client/src/operator/lookup.rs"]`) and uses the
public `ipc_exchange_fixture::{raw, session::{Session, Endpoint}, Number, Wait,
PieToken, TaskId, Contract}`. The exchange call is the real public
`Session::call`; only the call spelling changed
(`exchange::call::<tests::ContractNum>(&s, …)` to `s.call::<ContractNum>(…)`).
The finite-timeout, same-session `Forever`, no-second-push, `pushes == 1` and
ended-receiver assertions are unchanged. The existing seven operator-client
tests are untouched, so that suite has nine.

Source suite counts are `18 + 9 + 6 + 23 + 6 + 5 + 20 = 87`:
`namespace-layout` 18 includes ten production inline tests plus eight local
tests; `operator-client` 9; `startup-authority` 6; `supply` 23;
`operator-connection` 6; `terminal-connection` 5; `virtio-blk-coord` 20.
Together with the provider's 69 tests the split is `69 + 87 = 156`, preserving
the original `71 + 85`. `terminal-connection`'s five tests compile-check
`read_with`; they are not claimed as direct runtime coverage.

## Verification

All seven source suites ran with `--locked` and then `--locked --offline`
against `x86_64-unknown-linux-gnu`; all fourteen commands returned 0 and each
mode reported 87 tests. Raw logs and exits are under
`evidence/pr15/consumer/logs/`; lock and metadata records are under
`evidence/pr15/consumer/{locks,meta}/`.

RV64 `cargo check --workspace` passed locked and offline. `cargo build -p kernel
-p programs` produced 90 executables in each profile, locked and offline
(`--message-format=json`). The artifact table records 90 package/source/path/
hash rows per profile; every ELF has one `_start`, one
`rust_begin_unwind`, at most one `___rust_alloc`, a unique target name and a
unique hash. The table was captured from the pre-QEMU artifacts and its hashes
match the saved immutable kernel copies; normal debug and release copies were
saved before QEMU.

Four QEMU scenes passed with raw exit 0, capture status 0, one entry task 1 with
reason 0, natural halt, and real HART counts: product SMP4 and SMP8, accept
SMP4, and iommu-product-fault SMP4. Frames were 85→85, 153→153, 85→85 and
85→85. The three fed scenes reported `feed.result=ok`. The fault scene ordered
execution-domain stop → device `status=0 queue_enable=0` → IOMMU queues stopped
(`faults=127 last_did=8`) → release `frames=0 stopped=true`, with
`sentinel_held=true status_held=true`. The independent judge is
`evidence/pr15/consumer/qemu/judge_qemu.py` with its `independent-judge.json`;
the timeout was the default 60 s and no protective termination was used as
proof. No 1024 scenario was run.

After QEMU, the live release kernel cache hash differed from the saved copy.
This is recorded as an observation only; it is not attributed to a particular
command, and the saved copies were not rebuilt or re-run for it.

## Boundaries

Native host clock remains unsupported; the PR14 initial Native-clock exit 101
and the later compile-only gate (no Native success stub) remain recorded in
`migration-stage3-ipc-time-boundary.md`. The consumer's `with_clock` fixtures
passing is not Native runtime execution.

This round completes the IPC consumer extraction and its stated validation
matrix only. It does not claim the full Stage 4 matrix or overall Stage 3/4
completion. The provider's published `README.md`/`PROVENANCE.md` describe its
local validation history and were not rewritten to track this consumer state.
