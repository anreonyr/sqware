# Driver aggregate and Hub server extraction

This Source slice continues accepted PR52, commit
`6768bcfa50ce0c253f831e419ba3f7ebcdcc7431`, without resetting the stacked history.
The combined entry and selected Deployment belong to the vanllia composition
layer; this transitional consumer change remains in `anreonyr/sqware`.

## Repository ownership and fixed versions

`components/driver` selects public Driver
`f202872b28372799a56957d7e272abf6758cbc14`. Its real Git submodules select:

| Unit | Public repository | Full revision | Scope |
| --- | --- | --- | --- |
| Hub | `https://github.com/shuyra-arche/hub.git` | `daa23b2611fee0c503bc6bd31a16121d97698ad6` | declaration, API history, client, server and owned tests |
| Router | `https://github.com/shuyra-arche/router.git` | `94fec223f3c5df6c99a49527a65d016d88de3af4` | pure API prerequisite; client/server still Source |

Hub extends existing public history. Runtime API identity remains Hub
`cccef3f325198c8390007ae88b064bae0f21f93e`; it does not switch to a second local
API package. Source clients and Hub select canonical `driver-support` from
Driver `8e6de5a77aa52184bb6f7fbbcbc961f779e94629`. That older immutable support
bootstrap is deliberate: publishing it before Hub avoids a repository release
cycle. Only its support package is reachable. Current Driver aggregate support
bytes equal that canonical package, including the original Peer.

Driver owns its explicit pure Hub declaration/build set, fixed Unit versions,
shared support and joint acceptance contract. The aggregate does not own the
combined Source deployment or a global all-Units registry. Source keeps exactly
the existing declaration order and Deployment bodies, substitutes `hub::UNIT`,
and retains `shared::peer::Peer` call paths through a public support reexport.

Hub owns its build coordinates: manifest `server/Cargo.toml`, package
`hub-server`, binary/image `hub`, features `[]`, RV64 target, release profile and
default features enabled. Driver prefixes only `units/hub/`; Source prefixes
only `components/driver/units/hub/`. Actual Cargo executables supply the images.
The inventory remains 89 logical images: 85 Source binaries and four owned
targets (Hub, Pipe, Account, Terminal). This is not a crate or Unit count.

Root removed exactly 82 hash-verified originals: 44 Hub source/declaration files,
five Router API files, one Peer file and 32 Hub-owned test files. The public Hub
Unit retains 65 runtime cases plus 19 canonical API/coordination cases, with no
duplicated coordination suite. Source keeps mixed device recovery (8 Rust and
126 Python), seven system-shape cases, Router handoff and all composed feeds.
Every original system-shape assertion remains; the migrated roots are scanned
explicitly and missing submodules fail closed.

## Validation

Provider gates passed before consumer intake: clean public Hub builds and 84
host cases, actual Hub debug/release ELFs; Router API host/RV libraries; Driver
recursive clean public metadata, host and RV library builds. Public full SHAs
and original source/provenance maps are retained in their repositories.

Current Source validation passed 21 online and 21 locked/offline metadata jobs.
Tuple comparison uses `(name, version, source)` and permits only the intended
Hub/Router/Driver transitions. Actual graph checks preserve canonical API and
support identities, pure aggregate reachability and the existing fixed versions.
All 21 tested consumer locks are included; four previously ignored generated
locks are newly tracked so clean-checkout `--locked` commands have their inputs.

All 197 targeted host cases passed: 71 Rust and 126 Python. The first Python run
retains raw1 from four Unix-socket bind denials; the unchanged 126-case command
passed raw0 with the supported per-command network grant. No assertions changed.
Six representative Source binaries (Router, IOMMU, virtio-blk, Login, Shell,
System) built locked/offline in debug and release. All twelve actual Cargo ELFs
were preserved with byte/hash round trips, RISC-V machine/type, entry address and
exactly one startup, panic and allocator provider per executable.

Twenty new real QEMU runs passed raw0: terminal session, Shell smoke/edges/
shutdown, accept, system-fault, memory-transfer, IOMMU ruin/forced/fail-fence,
each at SMP4 and SMP8. The separate Root judge passed all20: exact Hart counts,
original scenario/feed judges, capture completion, one natural final halt and
final frame counts restored (85 to85 at4,153 to153 at8). Eighteen root tasks
exited zero; the two forced runs retained the required native self-doom
`0xfffffffe` only after confirmed retirement settlement, with doom/exit/reap/halt
ordering checked. Leaf fault exits retain their original scenario semantics.

The actual selected Hub image came from the discovered `group-4` cache, not an
assumed group. Its locked/offline Cargo proof reports every artifact fresh, the
exact selected Hub executable, unchanged bytes/mtime, canonical API/support
identities and one startup/panic/allocator. Its independent preserved ELF hash
is `de320d6809d465489582ad3939b131ad26649a2bb22a9dde687cde3a5b28e409`.
All functional Source inputs and21locks stayed frozen through these checks.

Kernel remains `3aa0a155f825fbf377fc8d04009472ca67cbbf8c`, System runtime
`9001af65f97501dfa490c9397c61d593c871136d`, System SDK/support
`5a792ff2d39901a970925b1a6d63206adf6c16d8`, image-builder
`17f294bae58ef5526a18fd6a88a88cc41850d4a6`, Pipe
`4fce76372beb6c5ab8567232516f156b3d1032eb`, Session
`f7c7542acf0bf010c1aac07d36d30a19e0591731`, Account
`7dc701facdbe6368fbc09f7429920ad49dacdfca` and Terminal server
`f07a0954f65472e9a16e878e52c5f1d34146bc64`.

## Scope and retained limitations

This accepts only the stated Hub/Driver consumer slice. Router runtime, RTC,
UART, PCI, IOMMU and virtio-blk servers and remaining Session/Shell work are
pending their own ownership and integration gates. Shell commands stay in one
delivery repository. No complete Driver/Cluster, stage3/stage4, all-89-image
matrix, 1024 long test or whole migration completion is claimed. Historical
full-health missing-initrd failures and ordinary-boot final-frame limitations
remain documented in earlier migration records; their old results are not new
Source validation.

Core boundary edits came only from this PR's independent genuine DeepSeek
namer/worker/reviewer group through normal file tools, with shell tools removed
and filesystem/approval policies retained. Root independently chose and ran
validation, checked publication/locks/ELFs, removed exact originals and accepted
the final tree. Raw exits and independent judges are separate cloud evidence in
`/workspace/evidence/hub/source-intake/`; that evidence is not a Git dependency.
