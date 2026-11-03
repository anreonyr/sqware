# Terminal public API/client contract

This slice continues Source `c96f4a4b5a6795fab899c95bf11c350c20ba2572`
(pipe Draft PR49). It publishes the previously Source-local Terminal contract
at https://github.com/shuyra-arche/terminal, fixed revision
`c447c0761835339a39efdf2ddd8101ebf80db15a`, then consumes that public version.
The existing stacked Drafts remain unmerged.

## Ownership

The provider owns `terminal-api`, `terminal-client`, five wire-codec goldens
and five connection tests. Source retains the Terminal server, UART,
canonical input/mode, UnitFile, selected Deployment, `prog-terminal` and its
image BUILD coordinates. This is an API/client cut that unlocks Account's
public `terminal_api` dependency, not a claim that the complete Terminal Unit
or all remaining Units have moved. No provider UnitFile, BUILD or server shell
was invented. There are still89 logical images:88 Source binaries plus the
existing provider-owned pipe image. This library cut adds no executable image.

The provider records the frozen Source SHA, original authors, hashes and
per-type/test mapping in `PROVENANCE.md`. The Source root has no project
LICENSE; none was invented. Twelve original API/client/connection-test files
were checked byte-for-byte against the frozen snapshot before removal; the
original bytes and evidence remain preserved outside Git.

API `Command`, `Reply`, `Input` and client `Terminal`, `Connection`,
`Foreground`, `Io`, `Read` now each have their own source file. Existing
`terminal_api::frame::*`, marks/interface exports and the five client root
reexports remain stable. Only split-internal `pub(super)` access was added;
there are no new public fields or wrapper types. Root's independent token
comparison finds every original production method/Drop body unchanged, and
API lib/marks and client debug files remain byte-identical.

## Behavioral contract

The terminal server remains session/canonical-mode and data-grant authority.
`Terminal::find` resolves SERVICE and the relative `terminal/attach` endpoint.
Connection IO uses the acknowledged seeds. The separate existing
`Io::injected(owner)` indexed-discovery path is deliberately unchanged.

Entry/BACK/root/pile release, command-proof revoke, failed-lend restoration,
foreground revoke/restore and Drop ordering are retained. `active` clears
only after successful restoration; existing Drop retry behavior is retained,
with no new retry. Control is polled before input, busy reads use the existing
deadline/pile wait, busy `try_write` returns false, and write/drain retain the
1000ms bounds. No protocol, permission, error or lifecycle changes were made.

Wire fields/order, Command25bytes, Reply33bytes, Input payload/strict boundary
checks, MAX256, opcodes and all legacy channel marks remain unchanged. The
interface ID remains `sqware.user.terminal.v1` despite repository naming.

The provider's connection host workspace projects its own canonical client
modules using real `#[path]` modules; no Source backlink or copied production
body is used. The fixture types were split without changing their behavior.
All ten original test bodies compare token-identical. Source removes only the
two Terminal-specific `device-api` cases, three `terminal` stream cases, and
the moved connection suite. All six Source canonical-mode cases and surviving
Router/mixed-provider registry assertions remain unchanged.

Source `system-shape` now checks public full Git pins and absence of local
SDK paths. Its old Terminal-internal predicates were relocated to checks on
the exact clean public provider checkout: mold interface, API/client dependency
whitelists, explicit system-client membership, and absence of protocol
dependency/paths. The evidence binds observed HEAD and clean state and is
reproducible with the Root `validate-provider.py ... shape` harness. Source
still checks `terminal_api::REGISTRY` in actual assembly.

## Fixed dependencies and validation

Kernel remains `3aa0a155f825fbf377fc8d04009472ca67cbbf8c`; System server
`9001af65f97501dfa490c9397c61d593c871136d`, client/support
`5a792ff2d39901a970925b1a6d63206adf6c16d8`, API
`9e5d63ca536a176583cf1b0054b4ecf7ddd88257`; image-builder
`17f294bae58ef5526a18fd6a88a88cc41850d4a6`; pipe
`4fce76372beb6c5ab8567232516f156b3d1032eb`. No other provider was repinned.

| Gate | Actual result on this slice |
| --- | --- |
| Provider local and fresh public checkout, locked/offline |10 host cases, raw0 |
| API/client RV libraries, debug/release, local and public |raw0 |
| Source affected host suites |54 passed across8 gates, raw0 |
| Source affected actual Cargo artifacts |8 ELFs: Terminal/Account/Login/Shell in both profiles; origins and unique startup/panic/allocator checked |
| Real Source product + original terminal/session.py feed,4/8 Harts |qtest raw0, independent judge0, natural completion; frames85→85 /153→153 |

The feed checks unknown user, wrong password, password-entry cancellation,
successful identity/session interaction and secret non-echo before EOF.
Provider mocks alone do not establish native capability cleanup; the real
Source-owned server run supplies that integration evidence. Both qtest/feed
criteria and the independent completion/capture/nonzero-frame judge remain.

Actual Cargo package sources resolve API and client from the same full public
Terminal revision. Tracked locks are compared by(name,version,source); only
Terminal package tuples change. Ignored locks either remain byte-identical to
the previous accepted cloud checkpoint or have their resolved package metadata
compared; full prior ignored-lock bytes were not retained, so this does not
claim a full byte-diff for those files. Runtime source/locks stayed frozen.

The6GiB guard stopped before8-Hart work after the4-Hart pass. Only precise
tagged, inactive compiler incremental-state directories were cleared. Source,
all prior/raw ELF evidence and current8 artifact hashes were preserved. The
same validated input tuple then resumed8-Hart; the4-Hart pass was not rerun or
carried across changed code. Metadata/CLI inventory establishes89 coordinates;
no blanket178-ELF, full Kernel or1024 rerun was done just for counting.

## Agent and evidence provenance

This PR uses a fresh genuine DeepSeek group, `opencode-go / deepseek-v4.1-flash`
through dsh0.2.1-rc.4: namer
`session-f13caee3-b4fc-4670-b707-4440548e9d6b`, sole worker
`session-a7cc7207-75c7-4150-8e49-a85891821ea6`, independent reviewer
`session-ee616306-13c4-4c9e-8daa-64d478616c41`. The prior PR groups remain
sealed; no instance/chat/session was archived or deleted. Shell registration
stayed disabled, FS sandbox/observation/approval policies stayed enforced, and
Root independently selected native checks, generated locks and handled Git.
No arbitrary model commands were relayed through the Root executor.

Cloud evidence is `/workspace/evidence/terminal`: snapshot/removal maps,
method/test-body preservation, actual Cargo metadata/artifacts, raw exits,
public clean-checkout checks, shape predicates, cache stops/resume and strict
runtime judges. This instance does not inherit oldhost caches/SIDs. Local
Vanllia relocation did not rename cloud paths or Git remotes.

The prior normal-boot idle CPU raw1, historical missing-initrd health sentinel
and negative-Face coverage distinction remain recorded, not reclassified as
passes. Ordinary boot still does not prove testing-only frame reclamation.
Account, remaining Terminal server/UART/Units, lisp/alloc-probe and Shell
adaptation remain future work. This slice does not claim overall migration or
all-tests health completion.
