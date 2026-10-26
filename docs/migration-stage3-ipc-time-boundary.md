# Stage 3: IPC time boundary preparation

Public source: `https://github.com/anreonyr/sqware.git` at
`90f2e85643985cd163ae02b33995acaec785d8e4` (PR36,
`codex/shuyra-schedule-extraction`). This round works on local branch
`codex/shuyra-ipc-time-boundary`. At validation time the branch was uncommitted
and unpublished; root publication supersedes that snapshot.

This is IPC source-seam preparation only. It does not create an IPC repository,
delete an IPC implementation, change a published provider, or change a pin.

## Functional contract

`ipc::time` remains a budget calculator. The clock is authoritative in `env` or
in a fixture; `Deadline` owns no clock state, resource, lock, retry, or rollback.
The non-generic `Deadline`, its two private fields, its derives, and the
`new`/`remaining` signatures and `Debug`/type appearance are unchanged.

Two public entry points now hold the only algorithms:

- `new_with_clock(wait, clock_ns)` calls `clock_ns` exactly once for every
  `Wait`, including `Forever`, and stores the returned nanosecond clock reading.
- `remaining_with_clock(self, clock_ns)` returns `Forever` without calling the
  clock. A finite wait, including a zero budget, calls it once and preserves
  the old floor conversion:
  `(now.saturating_sub(started) / 1_000_000) as usize`, followed by
  `budget_ms.saturating_sub(elapsed)`.

The old `new` and `remaining` delegate to these entry points with
`chrono::clock` passed as a function (not evaluated first). No callback is
stored, and no field, trait, native marker, `PhantomData`, cache, global state,
lock, catch, or host success stub was added. Clock panics propagate. The free
`deadline`/`remain` functions are byte-identical to the frozen source.

## Real sources and test migration

Three cross-owner test workspaces no longer project `crates/ipc/src/time.rs`:

- `programs/tests/virtio-blk-coord` (`dev-dependencies`)
- `programs/tests/terminal-connection` (`dependencies`)
- `programs/tests/operator-connection` (`dependencies`)

Each has `ipc_impl = { package = "ipc", path = "../../../crates/ipc" }` and one
local adapter file: `fixtures/time.rs` for virtio, `time.rs` for the other two.
Each adapter defines one `Deadline` tuple struct holding one real
`ipc_impl::time::Deadline`; it keeps `Clone`, `Copy`, `Debug`, `PartialEq`, and
`Eq`, and its `new`/`remaining` pass that suite's existing `chrono::clock` to
the real public `*_with_clock` mechanism. The suite clock sources are unchanged:
virtio CLOCK TLS `Cell`, terminal constant `0`, operator `BACKEND.now`.

The three suites keep their original 20 + 5 + 6 = 31 tests. Included
cross-owner production sources have zero free `deadline`/`remain` calls; the
adapters expose only `Deadline`. Terminal's five tests compile the `read_with`
path but do not directly run `read`/`read_with`, so this round claims no
terminal time-runtime coverage.

The IPC-owned host workspace (`crates/ipc/tests/host`) now depends on the real
package through `path = "../.."` (as `ipc_impl`, avoiding the `extern self as
ipc` projection) and has a separate `time_boundary` module. Its eight tests use
the real public `Deadline`, not a fixture wrapper, and cover: one clock read for
Forever/zero/finite `new`; zero reads for Forever `remaining`; one read for a
zero finite budget; backward-clock budget preservation; 999999/1000000/1000001
ns floor boundaries; exhaustion and `usize::MAX` saturation/cast order; natural
panic propagation from `new` and `remaining`; plus old traits, old function
signatures, and inference without an annotation. Expected values are explicit.

The original IPC-owned suites remain 20 + 17 + 23 + 3 = 63 tests; with the eight
new host tests the actual total is 71 there and 102 with the cross-owner suites.
All seven host suites pass both `--locked` and cached `--locked` with
`CARGO_NET_OFFLINE=true`.

## Exploratory check and host limitation

An early exploratory run of the new host test actually called the real
`Deadline::new`, so it executed the Native host clock; the host `env` ecall trap
is unsupported. The original tool result
`call_00_6utqwtv0vxxv7myp6ji07b8m` was 27 passed, 1 failed, exit 101. The test
was then narrowed to a compile-only signature/traits/inference gate; the core
Native clock gained no success stub and changed no behavior, and the
pre-existing 31 + 63 assertions were not modified. A second Cargo run passed
28/28, but its `tee /tmp/pr14/host-second.log` log write failed because `/tmp`
was not persistent across tool calls. The later persistent-path seven-suite
locked/offline matrix (14 commands) all returned real exit 0. Root preserved the
initial failure at
`evidence/pr14/host-tests/root-preserved-initial-native-probe-failure.log` and
maintains `root-raw-exit-and-exploratory-events.json`; neither is rewritten
here. This round is therefore not failure-free: the Native host clock remains
unsupported, while the real host mechanism passes through explicit clock
callbacks. The ABI probe never executes the Native clock.

## Locks and artifacts

The host `.gitignore` no longer ignores `Cargo.lock` (it keeps `/target/`), so
the new host lock is now a tracked candidate. The three existing suite locks
gained only the real `ipc` path package and, where absent, `resource`; the
previously ignored host lock is tracked with its full actual closure. Complete
`(name, version, source)` records show no existing record upgrade and no
name-only overwrite. The root `Cargo.lock` and every other existing lock/pin are
unchanged. Two other IPC-owned suite locks (session-exchange and
session-establish; same owner, previously ignored) remain ignored as before;
only their actual SHA-256 values are recorded for the future real IPC repository
extraction. No unrelated `.gitignore` was changed.

RV64 workspace check and kernel/programs debug and release builds pass locked
and cached offline with actual Cargo JSON. Each profile yields 90 executables:
one `shuyra` kernel plus 89 programs. All 90 paths and SHA-256 hashes are
recorded per profile; every ELF has one `_start`, one panic landing symbol, at
most one allocator shim, unique target names, and unique hashes. The normal
release kernel copy was saved before QEMU.

The old ABI evidence under `evidence/pr14/abi-baseline/` was not overwritten. A
separate post-change probe reused its sources and lock in an independent target
and emitted LLVM IR: public `Deadline` is still 24 bytes, align 8, and the old
signatures/traits compile. Native clock was not executed by the probe.

## QEMU and boundaries

Four new release scenes pass with independent judging: product terminal with
SMP 4 and 8, accept terminal with SMP 4, and `iommu-product-fault` with SMP 4
(EOF plus the existing semihosting path). All have real HART 4/8, unique entry
task 1 with reason 0, natural halt, zero capture status, and frames 85->85 or
153->153. The three fed scenes have `feed.result=ok`. The fault scene shows the
real domain stop, device `status=0 queue_enable=0`, IOMMU queues zero with
`faults=127 last_did=8`, release `frames=0 stopped=true`, and held sentinel
status. No protective termination is used as proof; timeout, CPU count, and
reclaim checks were not relaxed.

For the selected IPC dependency there is no implementation copy, patch, or
remap. The unrelated, pre-existing alloc-probe vendor patch was not evaluated
in this round.

Not verified or claimed: the Stage 4 full matrix, a real IPC repository
extraction, remaining Stage 3 work, and additional IOMMU/1024 scenarios. This
document does not claim overall Stage 3 or Stage 4 completion.
