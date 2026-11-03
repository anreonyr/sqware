# Pipe extraction and first independent Integration

This slice continues Source `f82ee2a9751b09563ca0f21577c212ab4470c8e4`
(image-builder Draft PR48), on top of System PR47 and Kernel PR46. It extracts
pipe declaration/API/client/server/tests into the public
https://github.com/shuyra-arche/pipe repository, then consumes one fixed version.
The existing stacked PRs remain unmerged.

| Component | Fixed public revision |
| --- | --- |
| Kernel | `3aa0a155f825fbf377fc8d04009472ca67cbbf8c` |
| System server | `9001af65f97501dfa490c9397c61d593c871136d` |
| System client/support | `5a792ff2d39901a970925b1a6d63206adf6c16d8` |
| System API | `9e5d63ca536a176583cf1b0054b4ecf7ddd88257` |
| image-builder | `17f294bae58ef5526a18fd6a88a88cc41850d4a6` |
| pipe | `4fce76372beb6c5ab8567232516f156b3d1032eb` |
| unit | `90396f0fda376e0693fb1b9553aa21864d241057` |
| stream | `ebd1c6db4823e28591650d967d7e8c8dd1939687` |

## Ownership and consumer boundary

The provider owns `pipe-api`, `pipe-client`, `pipe-declaration`, and
`pipe-server` (binary `pipe`). Source consumes public full Git revisions and
pins `components/pipe` to the same commit. Source still owns its actual
declaration list and selected Deployment. It uses provider `UNIT` and `BUILD`,
adding only the checkout manifest prefix `components/pipe/server/Cargo.toml`.
The image name, package, binary, features, target and profile remain separate.

There are still **89 logical images**: 88 binaries in Source's `programs`
package plus the provider-owned pipe server. `prog-pipe` is no longer a Source
target. Shell command images keep their catalog/supply contract; they do not
gain UnitFiles. No global registration, reverse Source dependency, local
override, private linker script, copied runtime or second implementation was
introduced. Execution/mold remain the single startup/panic/allocator/link source.

| Removed Source ownership | Provider ownership |
| --- | --- |
| `programs/src/service/pipe/api/` | `api/`, one wire data structure per file |
| `programs/src/service/pipe/client/` | `client/`, preserved public paths and Drop semantics |
| `programs/src/service/pipe/core.rs` | `server/src/ledger.rs`, `record.rs` |
| `programs/src/service/pipe/serve.rs` | server root/grant/lease/process/ended/run modules |
| service `mod.rs` / `main.rs` | server library and ELF entry |
| `crates/composition/src/units/pipe.rs` | declaration `UNIT` and owned image `BUILD` |
| Shell ledger test and old core projection | provider `tests/ledger_case.rs` |

All nine original files were checked against the frozen snapshot hashes before
removal. The provider records source SHA, mapping and original authorship in
`PROVENANCE.md`; the source snapshot had no project license, and no license was
invented. The original Shell ledger case keeps its name and every assertion.
Shell Plan/Job, adapters, direction/Launch/catalog tests and real feeds remain
Source-owned. CLI catalog checks now validate exact provider coordinates and
the other 88 Source coordinates, retaining uniqueness and scene exclusion.

ABI bytes/opcodes (including unused4), seven failure codes, capacity limits,
marks, codec quirks, import's ignored wait argument, busy notify/hush behavior,
authorization and lifecycle are unchanged. Ledger reservations last for the
pipe lifetime; live grants are separate facts. CLOSE remains sticky half-close,
not RELEASE or permission to rebind. Lost BIND replies gain no idempotence
promise. Owner cleanup remains recursive Kernel revoke.

## Independent real fixture

The provider's separate Integration owns four explicit UnitFiles over three
ELFs: Supervisor `pipe-root`, server `pipe`, and two declared transient roles
`pipe-probe`/`pipe-peer` sharing the probe ELF. Root uses public System Fixture
mint/embark and typed settle/supervise; it supplies least-privilege marked Hole
context/report/command channels and retains actual minted TaskIds. Every
control frame checks its source and length. There is no private Registry/Plan/
Loader access or Source composition dependency.

Using a declared peer preserves System's admission boundary. The initial
runtime-thread fixture was rejected by normal operator admission; no permission
was broadened to make it work. Static activation uses its actual sire contract,
not a guessed dynamic bootstrap carrier. The firmware is an explicit public
test input from frozen Source `f82ee2a.../SBI.bin`, SHA-256
`c53d58aa9ab17f95fa1af159f0bf05fad5c5622ebc0f1084463435fa6e5752b6`;
its provenance does not assert a new license.

Real tests cover invalid/repeated failed create then success, wrong-owner BIND
on an unreserved direction, exclusive roles, exact40-byte ring wrap transfer,
EOF, exact Closed and BrokenPipe, invalid-target/unknown-id errors, release
with live peer seeds, owner death with Kernel seed dead + server exact Dead +
Port exact Dead before global teardown, then fresh create/release.

Kernel's existing semihosting diagnostics account for native spawns/exits/reaps
and faults. Root/probe/peer have named zero exits; bootstrap tasks have numeric
zero exits. The resident pipe follows normal System Ruin retirement: native
Reap and explicit System retirement are required. That path emits no numeric
exit cause/reason, so it is recorded **unobserved**, never fabricated as zero.
Natural all-tasks halt and nonzero exact before==after frame counts remain
mandatory. A protective timeout/kill never counts as cleanup.

## Validation

All following results belong to this cloud instance and this slice:

| Gate | Actual result |
| --- | --- |
| Public clean checkout, committed locks, offline host tests | 12 passed, raw0 |
| Public API/client/declaration/server RV libraries and server debug/release | raw0 |
| Public Root/probe debug/release, host inventory and three-image capsule | raw0 |
| Public clean-checkout real Integration, 4/8 Harts | raw0 and independent judge0; frames85→85 /153→153 |
| Source workspace / CLI host | 18 /12 passed, raw0 |
| Source standalone host suites | 23 suites, 290 passed, raw0 |
| Source actual Cargo artifacts | 178 ELFs: 88+1 in both profiles; source/package/bin/features/profile and unique runtime checked |
| Source 10 embedded scenarios ×4/8 Harts | 20 raw0 and independent judge0; final frames85→85 /153→153 |
| Independent fixture with current Source System server9001af6, 4/8 Harts | raw0 and independent judge0; frames85→85 /153→153; test-manifest-only projection described below |

The embedded Source scenarios are product, accept, system-fault, Shell
smoke/edges/shutdown, memory-transfer and short iommu-ruin/forced/fail-fence. Existing
scenario-specific judges and final reclamation assertions stay in place;
Root independently checks capture, entry completion, halt and frames. Source
fault scenes and Shell's deliberate fail commands retain their expected leaf
failures rather than being mislabeled all-guest-zero runs. Eighteen cases have
entry exit0; `iommu-forced` at4/8 intentionally ends the Supervisor by self-doom
after confirmed retirement settlement, with observed native exit
`0xFFFFFFFE → Reap → halt`. Root's initial extra uniform-zero judge rejected that
expected path; its failure is retained. The corrected Root native judge and
the original scenario-specific qtest/forced lifecycle judge jointly require
the exact settled flow, native self-doom/cause/order, DMA/alpha-before-Driver
proof, capture0 and nonzero exact frame equality. No Source assertion changed.

The original clean public pipe fixture pins System server5a792. To additionally
test the exact current Source combination, a separate fresh public4fce checkout
keeps every Core/fixture Rust byte and Kernel3aa unchanged, changing only its
test consumer's `integration/root/Cargo.toml` server pin to9001af6 plus normal
lock resolution. That projection's host inventory, debug/release fixture ELFs,
three-image capsule and real4/8 runs passed. Its exact two-file delta is retained;
it is explicitly a test consumer projection, not a claimed clean published
fixture revision, private Core override or replacement for the clean-public run.

Shell smoke retains real pipelines, pause/resume/cancel, background/foreground
read and Ctrl-Z assertions. The separate normal-boot `idle.py` is a standalone
QMP/CPU test, not a feed module; Root's initial attempt to pass it as a feed
failed before its assertions and remains recorded. Correct standalone runs at
4/8 Harts both had raw exit1: paused-job28% /background-read33%, against the
unchanged host-CPU threshold of<20%.
Their protective termination proves neither natural completion nor frame
reclamation. A targeted comparison using the preserved pre-pipe product capsule
also failed at4 Harts (login39%); its8-Hart run instead failed earlier at router
startup NotReady and supplied no CPU measurement. These results do not convert
the current failures into passes or prove timing equivalence. The CPU sentinel
is an explicit remaining health limitation, separate from the pipe functional,
authorization, lifecycle and embedded final-frame contract; there is no all-tests-
green claim and no Kernel or threshold change to make it pass.

The first edges feed exposed a test race: finite emit had completed before its
receiver was closed. The repaired feed uses existing `prepare → close → start`
to establish causality, keeps the same producer/1MiB volume, exact completed1,
60s cap and final cleanup. An AST comparison proves every other statement is
unchanged. Other Shell feeds, all production code and locks stayed frozen.
The failed run remains recorded; it is not a passing result.

The first8-Hart iommu-ruin run also exposed a System test-support timing defect:
its caller's5s Control-delivery budget started before startup and the real
post-submit DMA request. The same DeepSeek worker repaired only the provider's
`test_support/device_ruin.rs`: spawn occurs after the exact controller sender/
payload check; the caller body,5s assertion, Ruin5000ms, State1000ms, grants and
real Rust Join are retained. Root proves the entire caller body byte-identical.
The Join stays on the supervisor via `Rc<RefCell<Option<Join<()>>>>`; it is joined
on success/error without a held borrow, and a never-spawned caller cannot pass.
No production Rust or API bytes changed. System server9001af6 extends5a792;
its manifest explicitly consumes the existing public client/support5a792 so
Source and pipe keep a single nominal client identity. The System gitlink thus
matches server9001af6, while client/support remain5a792 and API remains9e5.
Provider local and fresh public checkouts both passed29 host suites/332 tests
and RV test-support libraries in debug/release. The failed original run is
retained; current Source gates below use the newly fixed public server version.

Normal Cargo resolution changed only pipe package source/declaration and the
explicit System server package tuples;
no unrelated registry/Git upgrades occurred. A 6GiB disk guard stopped before
release when space fell below the floor. Only176 obsolete Cargo executable
cache copies, each byte-identical to preserved ELF evidence, were removed;
source/evidence/current artifacts were not deleted. Builds then resumed. The
server revision rebuild triggered the same floor after debug; only the precise
tagged System/Source host incremental-state directories were cleared. Prior178
ELF evidence inodes were protected before rebuild; all current178 ELF hashes
remained unchanged after runtime verification.

## Model and evidence provenance

This PR uses its own genuine DeepSeek group through dsh0.2.1-rc.4,
`opencode-go / deepseek-v4.1-flash`, with request/context routing independently
extracted, raw exits and completed turn ends saved. Worker
`session-37c16c7b-7da1-48f5-b106-4f0f7553266e` remained the sole implementation
writer; namer `session-9a51805d-0879-44ec-ac97-726c0bb01b19` and reviewer
`session-7b5fea8c-9138-407b-8175-5d380990a3b3` were read-only. Shell tool
registration was removed while FS sandbox/approval policies stayed active.
Root independently chose and executed native validation, lock resolution and Git
delivery; no arbitrary model shell commands were forwarded.

Cloud evidence is outside Git at `/workspace/evidence/pipe`: snapshot/removal
maps, safe group provenance, public release proof, normal lock tuples, frozen
Source hashes, actual Cargo artifact origins and preserved ELFs, raw/native
logs and independent judges. Old failed fixtures are archived. The previous
local/other-instance caches, SIDs and tests are not claimed as this instance's
results; the local Vanllia relocation did not rename cloud paths or Git remotes.

This slice does not rerun the old full Kernel matrix or1024 long tests. The
historical health missing-initrd sentinel remains a recorded failure, and
ordinary boot still does not establish testing-only final frame telemetry.
The earlier image-builder negative-lookup probe coverage distinction remains
as recorded in its migration document. Remaining Units, lisp/alloc-probe and
Source-owned Shell adaptation are not declared fully migrated by this trial.
