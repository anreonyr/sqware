# Stage 3: Kernel allocator boundary (PR20)

## Status

This document records the allocator mechanism extraction that lets a later Kernel
repository own the package. It is **not** completion of Stage 3 or Stage 4.

- Evidence root: `/home/anreonyr/Develop/shuyra/evidence/pr20/final`
- Source: `/home/anreonyr/Develop/shuyra/source`
- Branch: `codex/shuyra-kernel-allocator-boundary`
- Public base/PR42: `b022cc08d2b040fdff59b564f4f8c1066ccf4d00`
- Final precommit gate manifest: `evidence/pr20/final/source-manifest.tsv`
  (65 raw paths, sha256 `9074ae9f16f6cdf223336d92a38f5dde047410cb3de7ac73e57c6f897f05bf13`)
- This document is the only source-tree addition after that frozen gate; no production or
  test code changed after it.

## Boundary

`kernel/allocator/` is an ordinary `no_std` workspace member named `kernel-allocator`. It has
no own workspace/lock, no Kernel dependency, no start/panic/global allocator, and no global
instance registry. It owns the actual Bump/Frame/Block/Hybrid/Spare state and algorithms.

Kernel keeps the actual instance slots and initialization composition, board/hart facts,
native `SpinLock`/`OnceLock`/IRQ/`TrapGuard`/lockdep, `NoAllocation`, statistics/tags, trace
payload budget, portal/backend, and the only Kernel `#[global_allocator]`. Probe keeps the
arena, TLS-hart fixture, host `Mutex` lock binding, detector `#[global_allocator]`, shadow
oracle, and harness. All six Probe `include!` calls of Kernel-private allocator production
files were removed; Probe now binds the package.

The package depends only on `env` at canonical full Git rev
`d3000253cd5d13d466e91fafd4dbb27a5ec5aaae` (for `PAGE_SIZE`), `erra` 0.2, and `fack` 0.2.
The shared package and Probe reuse `env::PAGE_SIZE`; Kernel's existing architectural
`4096` constant is unchanged. There is no Kernel self-dependency or source projection.

## Safety boundary

- Lock is the approved unsafe unit-guard trait: native guard only, no cell/T argument.
- Kernel returns the actual `SpinLockGuard<'a, ()>` with Frame/Tally/Spare roles mapped to the
  existing Kernel level variants (10/12/13). Probe returns the existing shim's native
  `MutexGuard<'a, ()>` (levels 6/9/10).
- `Locked` and its `UnsafeCell` are private; every access is under the native guard for the
  whole closure.
- Bump initialization and all memory-grant effects are unsafe with explicit
  ownership/layout/freshness/no-overlap contracts. Host assembly is `pub(crate)`/private and
  trusted only through the harness `OnceLock` plus actual `Arena` ownership.
- `BlockEffect::release_frame` is unsafe; call sites use an owned pool page and exact layout.
- Hybrid returns the same stable `&'static (dyn Allocator + Sync)` Block/Frame instances.

Details: `evidence/pr20/final/unsafe-guard-audit.md` and
`evidence/pr20/final/source-lock-audit.json`.

## Initialization and live frontier

Kernel order is unchanged: Bump → portal Bump → statistics → Block → Frame → portal Hybrid →
Spare → record spare total. Probe keeps its existing Bump → Block → Frame → Spare order.

Frame initialization preserves the original outer `Box::try_new_uninit` before
`slot.write(FrameAllocator::new(...))`, then reserves freelist/pagemeta, runs the real
`install_frame_kinds` (which consumes Bump memory), re-reads the **live** Bump frontier, and
only then computes the final base and reserved holes. The owned regression
`frame_uses_post_install_frontier_and_never_returns_classified_or_reserved_bytes` proves a
classified page and a reserved hole are never delivered.

## Results

Baseline public source counts (already run by root before this work):

| Alias | Passed |
|---|---:|
| plain | 10 |
| smartalloc | 6 |
| mockalloc | 14 |
| dhat | 11 |
| mt | 4 |

Final precommit results:

- Probe: same five counts in both `--locked` (offline env unset) and `--locked --offline`,
  with test names identical to baseline. `10/6/14/11/4` are alias passes and overlap; they are
  not 45 unique tests.
- Allocation oracles unchanged: `INIT_BLOCKS=10`, `INIT_LIFETIME_LEAKS=10`,
  `INIT_BYTES_BUDGET=256 KiB`, `INIT_LIVE_BUDGET=64 KiB`; vendored smartalloc patch unchanged.
- Kernel-allocator host tests: frontier regression 1 passed both host modes; host/RV64 lib
  builds and both root workspace RV checks raw 0.
- Four final normal builds (debug/release × locked-only/offline): 90 bin artifacts each,
  180 canonical rows, 360 immediately captured records. Within each profile the two mode
  records match; debug `opt_level=0/debug_assertions=true`, release `opt_level=3/
  debug_assertions=false`; `_start` exactly one, panic handler exactly one,
  allocator-symbol count ≤1. Four exclusive pre-runtime Kernel copies are under
  `evidence/pr20/final/rv/normal-copies`; no copy was made after QEMU.
- Runtime: 26/26 accepted final entries pass independent judges — 18 health/mega (8 names ×
  SMP4/8 plus `memory_transfer_mega_leaf` × SMP4/8) and 8 guests (product SMP4/8, accept SMP4,
  iommu-product-fault SMP4, DMA invalidate/reset/release/reference SMP4).
- Guest frames are the expected 85 (product/accept/fault) and 153 (product SMP8); DMA reports
  87 → 87. Fault has two strict ordered chains, `faults=127 last_did=8`, sentinel held. DMA
  shows real transport rejection/injection removal with two retained references, automatic
  confirmation, same observed reference handle until release, final references=1, and natural
  halt. No protective kill or timeout extension.

Judge output: `evidence/pr20/final/independent-judge.json`.

Root independently checked the final source, host results, all 180 normal ELF hashes and
symbols, the 18 named Kernel tests, and all eight guest consoles. Records are under
`/home/anreonyr/.codex/worktrees/6299/sqware/target/migration-evidence/pr20/root-*-review.json`.
Full target-filtered offline Cargo graphs in that directory's `root-metadata/` contain
100 packages for the RV64 root and 36 for Probe's x86_64 host target. Both commands exit 0
and resolve one shared allocator and the canonical public `env` revision.

## Known failures, gaps, and superseded work

- The original `iommu-product-fault-smp4` run was interrupted by a wrapper Ctrl-C and has no
  child/QEMU raw exit. It is preserved untouched as interrupted. Only the retry directory
  `iommu-product-fault-smp4-retry` is terminal and judged.
- An earlier multi-case guest script was stopped because it unset
  `CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS` for DMA. The four final DMA runs explicitly set it
  to `true`; the three completed normal guests were not rerun.
- Earlier `evidence/pr20/rv` normal builds predate the final source gate and are
  superseded/stale; they are not final proof. Earlier `evidence/pr20/host-probe*` and
  `rv-visibility-fix` directories are retained pre-checkpoint runs.
- Some early failed build logs were overwritten before the final layout and cannot be
  reconstructed as standalone raw logs. Original worker JSONL files are retained under
  `/home/anreonyr/.codex/worktrees/6299/sqware/target/migration-evidence/pr20/`.
  `evidence/pr20/final/early-failure-transcript-excerpts.json` preserves 11 actual tool-result
  excerpts, explicitly labelled as transcripts rather than complete compiler logs.
- Worker-wide Probe metadata initially failed while trying to unpack an unrelated target's
  `r-efi` dependency into its read-only registry. Root's subsequent complete metadata
  command for the actual x86_64 target succeeded offline; the original failure remains recorded.
- Host tests cannot prove Kernel hardware IRQ/lockdep semantics; Kernel proof is the selected
  18 runtime tests. This is not a claim that the full health or initrd sentinel suite is green.
- The worker did not commit or push; Root performs reviewed publication after sealing its
  checkpoint. Kernel repository, System runtime, image-builder, pipe, and coordinator work
  remain incomplete. Stage 3/4 remain incomplete.

The actual worker provider/model was `opencode-go/deepseek-v4.1-flash`, session
`session-9542d523-7bba-4a23-ae8a-c4d2badace5d`, with working directory
`/home/anreonyr/Develop/shuyra`. Root verified the persisted request context.

## Source versus workspace artifacts

Accepted-gate source changes: 35 added paths (the new `kernel/allocator` package, Probe host
bindings, and the owned regression support), 11 deleted obsolete Kernel allocator
implementation files, 19 modified files (workspace/kernel/probe manifests, bindings, probe
host wiring/README, Kernel lock impl append). That is the accepted 65-path manifest. This
handoff adds only one documentation path on top of it (36 added paths total).

Evidence is outside Source Git and does not appear in that manifest: all files under
`/home/anreonyr/Develop/shuyra/evidence/pr20/**`, including build records, 180-row canonical
table, 26 runtime directories, judges, mapping, unsafe audit, and summary. Large files are
referenced, not copied into the small summary.
