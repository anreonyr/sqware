# Terminal server and UART contract extraction

This slice continues Source at
`f488db5538dc8dd8a530346d76e343708a3c5df0` (Draft PR50). It completes the
Terminal server move after that SDK-only slice. It does not declare all Units
or the overall migration complete, and it does not merge the existing stack.

## Published versions and ownership

| Component | Public repository | Full revision |
| --- | --- | --- |
| Terminal server and declaration | https://github.com/shuyra-arche/terminal | `f07a0954f65472e9a16e878e52c5f1d34146bc64` |
| Terminal API/client, unchanged | https://github.com/shuyra-arche/terminal | `c447c0761835339a39efdf2ddd8101ebf80db15a` |
| UART API/client | https://github.com/shuyra-arche/uart | `b690431496b0beaeb5fb7cafb42e60b4c8b38c9d` |
| Kernel, unchanged | https://github.com/shuyra-arche/kernel | `3aa0a155f825fbf377fc8d04009472ca67cbbf8c` |
| System server, unchanged | https://github.com/shuyra-arche/system | `9001af65f97501dfa490c9397c61d593c871136d` |
| System client/support, unchanged | https://github.com/shuyra-arche/system | `5a792ff2d39901a970925b1a6d63206adf6c16d8` |
| System API, unchanged | https://github.com/shuyra-arche/system | `9e5d63ca536a176583cf1b0054b4ecf7ddd88257` |

The existing image-builder `17f294bae58ef5526a18fd6a88a88cc41850d4a6` and
pipe `4fce76372beb6c5ab8567232516f156b3d1032eb` pins remain unchanged.
The new server/declaration extend Terminal's public SDK history. The SDK files
and their two original test suites remain byte-identical at31 files. Server
uses the public c447 API, keeping its identity aligned with Source clients.

| Original Source ownership | Public ownership |
| --- | --- |
| `driver/uart/core/frame.rs` | `uart-api::frame`, one `Bytes` type and the existing wire bounds/names |
| `driver/uart/client.rs` | `uart-client`, Console and discovery |
| `user/terminal/main.rs` | Terminal server library startup and its `terminal` ELF entry |
| `user/terminal/adapt/` | Terminal server scheduling, IO, attachment/authority and cleanup |
| `user/terminal/core/` | Terminal canonical input mode, split one type per file |
| `crates/composition/src/units/terminal.rs` | Pure `terminal-declaration::UNIT` and owned `BUILD` |
| Six `programs/tests/terminal` host cases | Provider `tests/canonical`, same test bodies |

Paths in this table are relative to `programs/src/` except the named
composition/test paths. Provider provenance records the frozen Source origins,
mapping and authors. No project license is invented or original history
rewritten. Root saved the15 original file hashes before adaptation and removed
only those exact originals plus the relocated host test's tracked lock after
public provider validation. The runtime `programs/tests/terminal/session.py`
remains in Source.

Source UART `main`, device/MMIO/IRQ handling, scheduling and UnitFile stay
Source-owned. Its client/frame module paths now reexport the single public
types. Source composition owns the actual declaration list and Deployment
selection. Terminal owns image/package/bin/features/target/profile coordinates;
Source supplies the `components/terminal/server/Cargo.toml` manifest prefix.
There are still89 logical image targets:87 Source binaries plus pipe and
Terminal. Libraries are not additional images or Units.

## Preserved runtime contract

Root independently compared39 moved method bodies, including UART, allowing
only the explicit boundary paths to public UART, `system_support::debug` and
`unit::READY_MARK`. The complete original Terminal UnitFile and Died24 tokens,
startup body and six canonical test bodies match. The execution linker adapter
matches the existing public pipe adapter. Internal split access is scoped to
sibling modules; no new console abstraction, registry or deployment was added.

UART keeps its raw nonempty byte encoding, MAX129, DRAIN_MAX64, names,
discovery order, waits and partial-failure result. Terminal keeps queue bounds,
schedule order, foreground/authority checks, grant rollback, reply filtering,
drop/revoke ordering and backpressure. Ctrl-D remains an input event, not an
automatic detach. Binary discovery/runtime exit reasons1/2 remain distinct
from declaration `E_TERMINAL: Died = 24`. Public mold/execution provide one
startup, panic handler, allocator and linker contract per executable.

## Actual cloud validation

The provider releases were reviewed, validated, committed and pushed before
Source intake. Independent public clean checkouts then repeated locked/offline
metadata, host tests and RISC-V debug/release builds. All final native commands
returned0. Host targets explicitly use `x86_64-unknown-linux-gnu`.

| Gate | Actual result |
| --- | --- |
| UART raw wire boundary suite | 6 passed, local and public clean checkout |
| Terminal API/connection/canonical suites | 5+5+6 passed, local and public clean checkout |
| Source affected host suites | 48 passed across7 suites, including12 CLI cases |
| UART libraries and Terminal libraries/server | RISC-V debug/release, locked/offline |
| Source affected binaries | UART, three rack probes, account, login, shell; debug/release |
| Corrected probe-watch binary | Separate debug/release builds |
| Artifact ownership | Actual Cargo executable JSON, RISC-V machine and unique runtime symbols |
| Source runtime Terminal ELF | Actual component package in the preexisting image cache; cached bytes unchanged during Cargo verification |
| Cargo source and locks | Public full source/rev checked;23 retained locks compared by `(name, version, source)` |

The70 host cases are distinct cases; local/public repetitions are not counted
twice. Lock comparison adds only UART API/client and Terminal declaration
packages where needed, with existing composition/programs dependency lists
updated. No unrelated version/checksum change was found. The relocated
canonical host workspace has its provider-owned lock.

Root ran a fresh frozen12-run set after the probe correction:

| Scene/feed | SMP4 | SMP8 |
| --- | --- | --- |
| product + original Terminal session feed | raw0 / judge0 | raw0 / judge0 |
| product + Shell smoke feed | raw0 / judge0 | raw0 / judge0 |
| accept, including Terminal handover/revocation and rack probes | raw0 / judge0 | raw0 / judge0 |
| system-fault | raw0 / judge0 | raw0 / judge0 |
| memory-transfer with its existing live driver | raw0 / judge0 | raw0 / judge0 |
| iommu-ruin short driver scenario | raw0 / judge0 | raw0 / judge0 |

Every final run passed its unchanged scene/feed judge and the additional Root
completion/capture/frame judge, with one real entry exit0, natural all-task
halt and exact nonzero physical frame equality:85/85 at4 Harts,153/153 at8.
Fault scenes and Shell fail commands retain their intentional leaf failures;
this is not a claim that every leaf exits0. Shell smoke exercises pipelines,
foreground IO, pause/resume, background reads, suspend and cancellation.
The original Terminal feed checks login failures, hidden password echo,
editing, interrupt, repeated sessions/principal identity and EOF.

## Retained failure and explicit coverage change

The first accept4 run returned outer raw1: `probe-watch` failed its post-unbind
`Face::tile_from(...AtMost1000)` expectation of `Unknown`. The embedded Kernel
scene passed and naturally reclaimed85/85 frames, but that does not make the
outer acceptance pass. The actual returned Fail/CallFail was not printed, so
neither a measured error variant nor a scheduler-timeout cause is claimed.
No isolated pre-PR baseline was run for this failure.

The probe file was unchanged from the base. A publication-wait lookup may
still return a positive-budget transport error; the published SDK preserves
that error. This was a third remaining permanent-negative probe of the class
documented in the image-builder slice. The same DeepSeek worker changed only
this negative block to one typed Relative RPC on the same Session/base/path
and original1000ms maximum, requiring exactly the UNKNOWN wire status.
Transport errors panic with their actual Debug variant; other statuses fail.
Every byte before/after that block, including all other watch, denied,
positive lookup/token, surviving-binding and cleanup assertions, is preserved.
Production, SDK retries, failure codes, permissions and timeouts did not change.

The new negative directly checks the server reply. It no longer covers the
exact end-to-end negative Face retry-exhaustion-to-Unknown wiring. Existing
retry host tests and positive Face runtime calls do not replace that coverage.
Independent DeepSeek review explicitly accepted this narrower seam with this
limitation. Root retained the first raw1 and reran all12 final scenarios without
carrying over the earlier two passes.

## Evidence and remaining work

New evidence lives in `/workspace/evidence/terminal-server`, including original
snapshots/hashes, public release proofs, native commands/stdout/stderr/raw
exits, exact byte/token comparisons, actual ELF records and copies,
`source-v2-judge.json`, full lock audit and the independent reviewer reports.
Final evidence review and Root delivery are recorded separately before push.

The fresh PR group uses genuine dsh `opencode-go/deepseek-v4.1-flash`, cwd
`/workspace`: namer `session-32b10ac6-53da-43aa-a1bc-848aa6e85aba`, sole worker
`session-b79d6148-6ece-4b76-85a6-c37c347cf5a6`, independent reviewer
`session-9089f45b-61e4-450e-bbbd-adefc7904fe7`. Routing is extracted from actual
persisted request metadata. Bash/pwsh/jobs registration stays disabled; FS
policies remain active. Core edits used the worker's file tools; Root selected
and executed native checks and Git delivery. Initial read-only inventories
were deliberately interrupted at raw130, then completed in the same SIDs with
raw0/completed. They are not API/authentication failures or implementation
success records. Final implementation/review turns are raw0/completed.

Root's first artifact-origin diagnostic assumed the wrong temporary group
index and failed its provenance check after Cargo raw0. Group order depends
on selected images. Its logs remain; the corrected check locates the
preexisting runtime artifact and verifies actual Cargo JSON, unchanged bytes
and symbols. No Source/runtime code changed for that diagnostic repair.

Disk checks stop new builds below6GiB. Only verified inactive compiler caches
were pruned; Source, unmerged work and ELF evidence remain. Old-instance files,
SIDs and historical test results are not claimed as this instance's results.
The old normal-idle CPU and missing-initrd health limits remain recorded in
prior migration documents. Ordinary boot does not prove testing-only final
frames. This slice did not rerun the full Kernel/89-image matrix or1024 long
tests. Account and the remaining servers/Units, lisp/alloc-probe and Source
Shell adaptation remain future work.
