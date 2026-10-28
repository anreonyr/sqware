# Stage 3: Hub API extraction

Status: consumer prerequisite for the Hub API extraction. The public Hub API is
published and every Source consumer now resolves it by full Git SHA. This is
not Stage 3/4 completion: Hub runtime/client implementation, the full Unit migration,
Kernel/System runtime implementation, image-builder implementation and the remaining pipe /
pause / resume / cancel / memory-driver matrix are out of scope here.

## Published provider

- Repository: `https://github.com/shuyra-arche/hub.git`
- Branch: `codex/source-snapshot`
- Full SHA: `cccef3f325198c8390007ae88b064bae0f21f93e`
- Crate: `hub-api` 0.1.0, edition 2024
- Cargo source identity:
  `git+https://github.com/shuyra-arche/hub.git?rev=cccef3f325198c8390007ae88b064bae0f21f93e#cccef3f325198c8390007ae88b064bae0f21f93e`

The canonical companion pins are unchanged:

| Crate | Source revision |
| --- | --- |
| `env` | `d3000253cd5d13d466e91fafd4dbb27a5ec5aaae` |
| `wire` | `0ff77dc4573fa2620d2fe995a18c01949d9c7e57` |
| `mold` | `f52501f4c91b686eded8b4bcd743b4e96084b7f9` |
| `system-api` | `9e5d63ca536a176583cf1b0054b4ecf7ddd88257` |

## Source changes

The old `programs/src/service/hub/api` workspace member was removed. The
remaining 14 direct consumers resolve only the public Git source above; there
is no branch, tag, local path, patch replacement or source export.

| Consumer manifest |
| --- |
| `programs/Cargo.toml` |
| `crates/composition/Cargo.toml` |
| `programs/src/service/hub/client/Cargo.toml` |
| `programs/src/driver/iommu/api/Cargo.toml` |
| `kernel/tests/dma-confirmation/Cargo.toml` |
| `programs/tests/capability-import/Cargo.toml` |
| `programs/tests/device-api/Cargo.toml` |
| `programs/tests/device-recovery/Cargo.toml` |
| `programs/tests/hub-handoff/Cargo.toml` |
| `programs/tests/interface-marks/Cargo.toml` |
| `programs/tests/iommu-pages/Cargo.toml` |
| `programs/tests/retention/Cargo.toml` |
| `programs/tests/supply/Cargo.toml` |
| `programs/tests/virtio-blk-coord/Cargo.toml` |

Removed implementation and moved suites:

- `programs/src/service/hub/api/`: 20 tracked files (19 Rust files plus
  `Cargo.toml`).
- `programs/tests/hub-api/`: three tracked files; its ignored `Cargo.lock`
  was also removed.
- `programs/tests/hub-coordination/`: five tracked files, including its
  tracked `Cargo.lock`.
- Total tracked deletions: 28. Before this delivery document is staged, 1802
  tracked files remain; the final Source commit contains 1803.

The Hub runtime (`programs/src/service/hub/{core,serve,...}`), Hub client and
all other APIs remain in Source. No UnitFile or replacement API was created.
No Kernel test was relocated and no Hub constant was copied into Source.

## Locks

Cargo performed minimal resolution. Each of the 11 remaining tracked
Hub-bearing locks gained exactly one line, the public Git `hub-api` source:

- `Cargo.lock`
- `crates/image/Cargo.lock`
- `kernel/tests/dma-confirmation/Cargo.lock`
- `programs/tests/{capability-import,device-recovery,hub-handoff,iommu-pages,retention,supply,virtio-blk-coord}/Cargo.lock`
- `tools/shuyra-image/Cargo.lock`

The tracked `programs/tests/hub-coordination/Cargo.lock` was removed with its
suite. Two ignored but live test locks were also updated by Cargo:
`programs/tests/device-api/Cargo.lock` and
`programs/tests/interface-marks/Cargo.lock`.

Inventory: 52 tracked locks at the base, 51 after the move, 11 modified,
1 deleted, 40 byte-identical. No package version, checksum, source of another
dependency, or unrelated dependency field drifted. There are no new
overrides and alloc-probe's existing vendored smartalloc patch is untouched.

## Test ownership

The provider owns the 14 legacy test functions: four from
`programs/tests/hub-api/lib.rs` and ten from
`programs/tests/hub-coordination/{tests.rs,device_key.rs}`. Their bodies were
copied unchanged into `tests/hub-api` and `tests/hub-coordination` in the
published provider. The provider also owns four source-contract cases and one
compile-only public-path case (19 total).

Four API-owned assertions were moved out of Source `system-shape` and are now
enforced in `tests/hub-api/tests/source_contract.rs`:

| Source assertion (base) | Provider owner | Intended check |
| --- | --- | --- |
| `system-shape/lib.rs:209` | `coordination_api_has_no_iova_ownership` | `api/src/coord.rs` has no IOVA. |
| `system-shape/lib.rs:255` | `dma_claim_has_no_requester_trust_shape` | The split `frame/dma` module has no requester/sensor trust field. |
| `system-shape/lib.rs:575` | `hub_api_dependencies_stay_env_wire_mold_system_api` | Hub API depends only on env/wire/mold/system-api. |
| `system-shape/lib.rs:710` | `hub_api_generates_interface_metadata` | `api/src/lib.rs` contains `mold::interface`. |

The remaining Source system/shape assertions stay in Source. The
`system-shape` suite still has 21 test functions; only the four old-source
reads were removed, plus the Hub API entry in the local API dependency loop.
No assertion was deleted to change a verdict: each check now has a real
provider owner with an actual passing case.

## Verification

Accepted earlier baselines are not rerun under a wrong CWD: Source 4 API /
10 coordination / 21 system-shape, and provider 14 legacy + 4 source-contract
+ 1 public-path, both locked and locked-offline.

Remaining Source host suites, both `--locked` and `--locked --offline`, all
passed:

| Suite | Tests |
| --- | --- |
| `kernel/tests/dma-confirmation` | 6 |
| `programs/tests/capability-import` | 8 |
| `programs/tests/device-recovery` | 8 |
| `programs/tests/device-api` | 5 |
| `programs/tests/hub-handoff` | 21 |
| `programs/tests/interface-marks` | 5 |
| `programs/tests/iommu-pages` | 52 |
| `programs/tests/retention` | 45 |
| `programs/tests/supply` | 23 |
| `programs/tests/virtio-blk-coord` | 20 |
| `programs/tests/system-shape` | 21 |
| **Total** | **214** |

`crates/image` tests: 25 passed in both modes. `tools/shuyra-image` builds in
both modes. The root workspace RV64 check passes in debug and release, locked
and locked-offline. Target-filtered `cargo metadata --locked --offline`
passed for 14 build roots; every live `hub-api` package is 0.1.0 with the
exact public source above, and the four companion pins are unchanged.

## Final normal builds

Four builds of `-p kernel -p programs --bins`:

| Build | Artifacts | Profile |
| --- | --- | --- |
| debug, locked | 90 (1 kernel + 89 programs) | opt-level 0, debug-assertions true |
| debug, locked-offline | 90 | opt-level 0, debug-assertions true |
| release, locked | 90 | opt-level 3, debug-assertions false |
| release, locked-offline | 90 | opt-level 3, debug-assertions false |

`CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS` was explicitly unset. Cargo JSON
artifacts were captured immediately after each build, before the next: 360
separate records with actual `executable`, `package_id`, `manifest_path`,
`src_path`, profile and SHA-256. The canonical comparison has 180 rows
(90 artifacts × 2 profiles); all hashes, sizes and profiles match between
locked and offline modes.

Four separately named Kernel copies were created before the first guest:

| Build | Kernel SHA-256 |
| --- | --- |
| debug, locked | `6cb7d739c38ca4139d5490a5805127d615672c8071859d1d31aca0f2890c5f9f` |
| debug, offline | `6cb7d739c38ca4139d5490a5805127d615672c8071859d1d31aca0f2890c5f9f` |
| release, locked | `3b8fec2c47dea09a7806a40cc1db04d0e1ad121917a711ac0f232d80e95033d7` |
| release, offline | `3b8fec2c47dea09a7806a40cc1db04d0e1ad121917a711ac0f232d80e95033d7` |

Exact `llvm-nm --defined-only --demangle` counts on all four copies:
`_start=1`, `__rustc::rust_begin_unwind=1`, `__rustc::__rust_alloc=1`.
No broad panic/allocator substring count was used.
Root also checked all 180 final Kernel/program ELFs independently: startup and
panic counts are exactly one, allocator count is at most one, and all 360
separately captured mode hashes agree by profile.

Root checked the four capture calls in their original worker transcript (rows
292/302/311/320), all before the first guest call (row 398). Their file metadata
change times (ctime) precede the next build and the first guest at 15:48:01 +08:00.
The initial capture helper used permissive copying; no overwrite occurred.
Root subsequently hardened future copy/record creation to refuse existing files,
without recopying any accepted ELF.

## Guest scenes

Eight new real QEMU scenes ran from the final Source. Normal scenes unset
`CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS`; the four DMA scenes set it to
`true` and used `--features semihosting,dma-confirmation-scene -- scene`.
Every scene used a unique `TRACE_OUT`, the canonical scripts, and the real
PCI endpoint where required.

| Scene | HARTs | Entry TID | Frames | Raw | Judge |
| --- | --- | --- | --- | --- | --- |
| `product` + `terminal/session.py` | 4 | 1 | 85 → 85 | 0 | pass |
| `product` + `terminal/session.py` | 8 | 1 | 153 → 153 | 0 | pass |
| `accept` + `terminal/session.py` | 4 | 1 | 85 → 85 | 0 | pass |
| `iommu-product-fault` | 4 | 1 | 85 → 85 | 0 | pass |
| `iommu-fail-invalidate` | 4 | 10 | 87 → 87 | 0 | pass |
| `iommu-fail-reset` | 4 | 10 | 87 → 87 | 0 | pass |
| `iommu-fail-release` | 4 | 10 | 87 → 87 | 0 | pass |
| `iommu-fail-reference` | 4 | 10 | 87 → 87 | 0 | pass |

The independent judge rechecked raw child exit, capture status, feed result,
firmware/hart count, entry TID, all task exit reasons, frame allocation,
natural halt and the expected completion line. For the four DMA scenes it also
ran the canonical `confirmation.py` against the captured console and
`sqware-diagnose.jsonl`:

- Each actual failure retained two IOMMU references, removed the injection,
  retried the same binding and preserved the original failure.
- Invalid/reset/release chains stop the domain, invalidate with six frames
  held, turn endpoint bus-master off, reset status/queue, stop IOMMU queues,
  release with frames=0, detach, and close the Hub binding.
- The reference scene observed a real handle (112), rejected release while
  still owned, removed the injection, then released to one reference.
- `iommu-product-fault` preserved the stale-IOVA sentinel/status (both true)
  and recorded `faults=127 last_did=8`.

No protective kill, hand input, assertion relaxation, 1024-generation soak or
timeout increase was used. The previous 18 allocator-health cases were not
rerun; they are unchanged and are not PR21 results.

## Evidence

`/home/anreonyr/Develop/shuyra/evidence/pr21/consumer/` contains:

- `Sourcecheckpoint.json`, `source-files.sha256`,
  `source-build-inputs.sha256`, `source-status.txt`
- `locks/` (Cargo metadata lock updates)
- `host/`, `rv/`, `metadata/`, `closure-audit.json`
- `final-builds/{debug,release}-{locked,offline}/{artifacts.json,build.jsonl,kernel.elf}`
- `final-builds/canonical.json`, `nm/`
- `qemu/<scene>/` consoles, feed logs, diagnose files, raw exits, `judge.json`
- `summary.json`, `checkpoint.md`, `MANIFEST.sha256`

Large verification logs stay outside the Source tree.

## Limits and known failures

- This checkpoint stops at the Hub API consumer prerequisite. It does not
  change Kernel/System runtime or the image builder, does not complete the
  Hub Unit migration, and does not complete Stage 3/4. The Shell/pipe,
  pause/resume/cancel and memory-driver matrix remain for later work.
- The worker performed no commit, push, or PR creation. Root owns final
  acceptance and publication; coordinator handoff remains deferred until Stage 4.
- Source retained its full Git history. No license is invented; the Source
  still has no tracked license.
- The previous consumer attempt (`35789`) exited 1 before any tool call on a
  provider rate limit. This retry preserved that fact; there was no final
  runtime or test failure.
- An earlier local judge revision had path/regex bugs in its own checks; it
  was corrected before the final judge and did not alter scene evidence or
  source.
- Independent review found the fault judge counted events without enforcing
  both full chains. Root added ordered checks and reran only the analyzer
  against the unchanged consoles; all eight scenes still pass.
- After the successful source/build/runtime checks, worker invocation 61867
  exited 1 on a transport connection error. A same-session terminal-handoff
  retry (52745) also exited 1 before any tool call. These are worker transport
  exits, separate from the recorded build/test/scene exits. Root sealed the
  checkpoint and completed evidence/doc finalization; source implementation
  and test assertions were unchanged.
