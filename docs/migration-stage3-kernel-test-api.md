# Stage 3: kernel test API fixture preparation

Public source: `https://github.com/anreonyr/sqware.git` at
`f16e11a235e6b06f3919a20fcf434c7e3ee21790` (PR39,
`codex/shuyra-stream-extraction`). This round works on local branch
`codex/shuyra-kernel-test-api`. The validation below records the pre-commit
worktree against that public baseline.

External evidence root (outside Source):
`/home/anreonyr/Develop/shuyra/evidence/pr17/`.

This is Kernel-owned fixture-wire preparation only. It does not create a
Kernel repository, coordinator, or published provider, and it does not claim
Stage 3/4 completion.

## Functional contract

`kernel-test-api` owns only the pure control-wire formats shared by the Fresh,
Race, and DMA confirmation fixtures: fixed VAs, tags, byte offsets, mode and
state values, and the 64-byte DMA command encoding. It holds no state
authority.

The original authorities are unmoved: controllers write the original
requests; the real Kernel hooks read the actual Caller/Task/Space/token and
physical frames; peers report the original coordinates. A request is not an
authorization. The Kernel's atomic state, hook gates, once consumption,
bounded waits, injections, real objects, and cleanup remain in place.

Programs actors, shared-page allocation, mapping, final closure, and
deployments are unmoved. This round adds no retention or allocator change, no
whole-allocation probe split, no fake Kernel host stub, private-source export
macro, new state, wrapper, lock, algorithm, UnitFile, or server.

## Moves and imports

The three wire files moved byte-identically:

| source at f16 | destination | SHA-256 |
|---|---|---|
| `programs/src/harness/probe/memory_transfer_fresh/handshake.rs` | `kernel/tests/api/src/memory_transfer_fresh.rs` | `8d061d6b319a801507d0422610e2866b8206c0ba8b47343b2dade943bf027c4a` |
| `programs/src/harness/probe/memory_transfer_race/handshake.rs` | `kernel/tests/api/src/memory_transfer_race.rs` | `3f1e5e75760b58d9b2de512007857dcb68de731e4485a658ceb84e976f2a58ae` |
| `programs/src/harness/probe/device_confirmation/injection.rs` | `kernel/tests/api/src/device_confirmation.rs` | `9d04dcb57c2fd5c3fba264176ac4bcf620c6a6f31a48b75f0865d8359c47675d` |

All three destination hashes equal both the frozen mapping hashes and the
source blobs. The old paths are absent; there are no forwarding files or
duplicate moved bodies.

Seven import sites now use normal crate aliases instead of `#[path]` module
declarations:

- `programs/src/harness/probe/memory_transfer_fresh.rs`,
  `memory_transfer_race.rs`, and `device_confirmation.rs`;
- `kernel/tests/health/fresh_scene.rs`, `race_scene.rs`, and
  `dma_confirmation.rs`;
- `kernel/tests/dma-confirmation/lib.rs`.

The aliases are `use kernel_test_api::memory_transfer_fresh as handshake`,
`...::memory_transfer_race as handshake`, and
`...::device_confirmation as injection`. Child `super::handshake` /
`super::injection` calls, function bodies, assertions, and `cfg` gates are
unchanged. Only the old-path/include comments in `probe/mod.rs` and the fresh
and race parents were locally corrected.

## Cargo ownership

`kernel/tests/api/Cargo.toml` is a normal workspace package:
`kernel-test-api 0.1.0`, `version.workspace = true`,
`edition.workspace = true`, no dependencies, features, build script, runtime,
own workspace, or own lock. `src/lib.rs` contains only `#![no_std]` and the
three public modules. The root workspace explicitly lists
`kernel/tests/api`.

The Kernel's dependency is optional and normal:
`kernel-test-api = { path = "tests/api", optional = true }`. Only the three
existing `fresh-scene`, `race-scene`, and `dma-confirmation-scene` features add
`dep:kernel-test-api`; `default = []` and every other gate are unchanged.
Programs has a normal path dependency; the DMA host fixture has a dev
dependency at `../api` and keeps its independent `[workspace]`.

`tests/abi.rs` adds two independent literal-vector gates: the complete Fresh
and Race offset/WORDS/VA/TAG/mode/state layouts, and the 64-byte DMA command
with distinct multibyte fields, both boolean encodings, the tag, and
little-endian byte order. The test file reads no implementation to form its
expected values.

## Locks

Only `Cargo.lock` and `kernel/tests/dma-confirmation/Cargo.lock` changed.
Full package-record comparison against `git show f16:Cargo.lock` allows exactly
one new `kernel-test-api 0.1.0` record per lock and exactly the parent edges
`kernel`/`programs` in the root lock and `dma-confirmation-tests` in the DMA
lock. No existing non-dependency field, version, checksum, or source pin
changed. Every other tracked lock is byte-identical; ignored local locks are
recorded separately. The Stream `ebd1c6db...`, IPC `29ad97dc...`, and System
API pins are untouched.

## Verification results

- API host tests: 2 passed, 0 failed (`--locked --offline`); the root-owned
  distinct `--locked`-only supplement also reports 2 passed. Its raw records
  are `host/root-api-locked.*` and
  `/home/anreonyr/.codex/worktrees/6299/sqware/target/migration-evidence/pr17/root-supplemental-host-locked-review.json`.
- DMA host test: the original 6 tests pass unchanged (`--locked --offline`);
  the same root supplement reports the required distinct `--locked`-only run
  as 6 passed (`host/root-dma-host-locked.*`).
- API host and RV64 `--lib` builds, Kernel default lib check, three feature lib
  checks, Programs lib check, and the root workspace RV64 check all exit 0 in
  both the original `--locked --offline` records and the new distinct
  `--locked`-only records under `rv/locked-only/`.
- The default `cargo tree -p kernel` contains no `kernel-test-api`; each of the
  three scene features activates it exactly once.
- RV64 debug and release builds produced 90 executables per profile (one
  `shuyra` kernel plus 89 programs), 180 rows total. Every ELF has one
  `_start`, one panic landing symbol, at most one allocator shim, and all
  hashes are distinct. The normal kernel copies were frozen before the first
  QEMU run; their hashes are `e480ac1c...` (debug) and `85aaf6b5...` (release).
  After the guest variants, a fresh combined `--locked`-only normal build
  reproduced the original 180-row table exactly: the new table SHA-256 equals
  the original table SHA-256, all 180 paths/packages/hashes match, and both
  default Kernel hashes match. The original table and pre-QEMU copies were not
  overwritten.

Seven real release scenes pass with the original scene drivers and judges and
`CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true`, `CARGO_NET_OFFLINE=true`, and
`CARGO_TARGET_DIR` unset. The final Fresh entry is the canonical source-runner
run in `qemu/fresh4-canonical/`; the earlier `qemu/fresh4/` run is a separate
diagnostic temporary run and is not the final gate:

- Canonical Fresh SMP 4: frames 87 -> 87; waiter queued/woke once, late reader never
  waited, both readers materialized zero pages. The Fresh driver resolves its
  diagnose file from `capture.parent.parent`, so that run staged `TRACE_OUT`
  one level under Source and moved the trace into the evidence directory
  afterwards.
- Race SMP 4: frames 87 -> 87; peer registered once, the foreign relation held
  the source page, the moved carrier read back equal, teardown 2 -> 1 relations
  and 1 -> 0 foreign PTE.
- Race SMP 8: same checks, frames 155 -> 155.
- DMA SMP 4: `iommu-fail-invalidate`, `-reset`, `-release`, and `-reference`
  each report a real transport/reference rejection, two retained IOMMU
  references, the same binding retry, the final IOMMU release, and frames
  87 -> 87.

All seven consoles report the actual HART count 4 or 8, one real entry, natural
halt, and strict final frame equality. No protection kill is used as proof.

## Canonical runner fix and baseline failure

The original `scripts/qtest.nu` at f16 uses two `//` comment lines in its Fresh
completion block (lines 542-543). Nushell 0.116.1 parses `//` as an external
command, so that wrapper exits 1 even though the embedded scene passed. The
real scene console, driver result, and pending summary all show success at
87 -> 87 frames. The original failed run is preserved under
`evidence/pr17/qemu/fresh4-original-runner-bug/` and remains a failed raw exit
1. It is not relabelled green.

The canonical source runner now differs from f16 by exactly those two comment
markers (`//` -> `#`); no runner logic, predicate, timeout, feature, or feed
changed. The canonical source Fresh run is exit 0 and is the final Fresh gate.
An earlier evidence-root corrected copy and the `qemu/fresh4/` run remain only
as diagnostic temporary evidence. The final matrix therefore does not claim
that every command ever run exited 0.

The normal copies were made before the first QEMU run and their original
hashes still match after the canonical Fresh rebuild. The current target-cache
release `shuyra` hash differs from the saved normal image. Normal production
artifact evidence uses the saved copies and the recorded normal builds;
scene builds use release with debug assertions and explicit scene features.
No extra rebuild was performed for the post-scene cache observation.

## Unverified boundaries

Standalone Kernel extraction remains blocked by the real IOMMU hold mechanism,
the allocator mechanism boundary (the allocation probe remains legacy), and
public IOMMU/PCI API packages for DMA host edges. Kernel-owned retention and
private-object integration tests stay in Kernel; the Programs retention suite
and deferred allocation probe were not moved.

No 1024-scene/full-health/default-sentinel all-green claim is made, and no
Native Kernel host success is claimed.
