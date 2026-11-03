# System runtime extraction

This slice continues Source `a29707fb83191647c63cc9ccfef1e8721bbe8dad`
(PR46). Kernel remains `3aa0a155f825fbf377fc8d04009472ca67cbbf8c`.
It extends the public system provider's existing API history at
`9e5d63ca536a176583cf1b0054b4ecf7ddd88257`.

Source owns the actual composition, ELF entry, declaration collection and
Deployment selection. `system-server::run` accepts Boot, explicit declarations
and the selected Deployment. It contains the runtime body; it does not select a
global composition. The server library supplies no ELF entry, panic handler or
allocator. The existing system API stays at its old revision, matching unit's
API identity. Client, server and support use one separately fixed provider
revision, `5a792ff2d39901a970925b1a6d63206adf6c16d8`.
`components/system` directly records that provider as a gitlink.

Neutral system-support owns boot/catalog/machine parsing, logging, timing and
tick calibration shared by Source programs and the server. Source passes its
PLIC class. The optional test-support feature owns original private runtime
fixtures and scenarios. Source keeps composition and account/terminal/login,
UART and driver policy through typed arguments and callbacks. Private Registry,
Control, Loader, Plan and raw resource containers remain provider internals.
AccountRig owns the original account-specific plan and locals; no second plan
or runtime supervisor is introduced. The programs package enables test-support
for its existing probes, so Cargo feature unification also compiles it for
other bins in that package; this is not per-bin feature isolation.

Provider `MIGRATION-MAP.json`, `PROVENANCE.md` and `TEST-OWNERSHIP.md` record
original paths/hashes, attribution/license status and mixed assertion ownership.
24 runtime-owned Source host suites move to the provider. Mixed system-shape,
control-rpc and capability-import suites retain their Source composition,
account and Hub assertions while runtime/watch assertions move to the provider.
Image catalog tests call the published support API instead of projecting a
removed Source implementation. Old implementation files are deleted only after
the provider is publicly published. Source system/main.rs remains the entry.

## Validation record

All records below belong to this cloud instance. Older Kernel acceptance is
historical context, not validation of this source tree. Raw logs and independent
judgements are preserved under `/workspace/evidence/system`.

- Genuine DeepSeek file write/read/edit/read probe completed with raw 0. An
  out-of-root write was rejected and Root verified unchanged fixture bytes.
  Shell tools were removed from the model catalog while filesystem and approval
  policies remained enabled. Root selected and executed validation and Git.
- Independent namer, sole worker and reviewer used distinct DeepSeek sessions,
  exact provider `opencode-go`, model `deepseek-v4.1-flash`. All launches used
  `/workspace`; the worker selected `/workspace/sqware` in turn 1, before its
  first write in turn 4, and retained that agent cwd for subsequent rework.
- Provider RISC-V debug/release checks and host library 23 tests passed, including
  test-support, locked/offline. API-owned suites passed 18 tests. The original
  migrated owned suites passed 280 tests; six new retry regressions bring that
  set to 286, while mixed provider halves retain 31 tests.
- Independent public checkout at `5a792ff2d39901a970925b1a6d63206adf6c16d8`
  passed 29 directly discovered host suites / 332 tests, locked/offline, plus
  server library 23 tests with test-support and a release RISC-V check.
  Its first run correctly failed because four ignored lockfiles had not been
  published; those generated locks were subsequently committed and revalidated.
- Remaining Source host suites passed 23 suites / 291 tests, raw 0. The initial
  all-bin compile found a missing device-ruin request re-export; the initial
  image test compile found an obsolete catalog source projection. Both failures
  remain recorded alongside subsequent validation.
- All 89 Source binaries build in debug and release, locked/offline raw 0.
  Root records 178 ELF hashes and checks one startup and panic symbol per ELF,
  with at most one allocator symbol (unused allocators are linker-omitted).
  Source workspace host tests pass 18 tests. Image tests pass 25 tests; the first
  repaired test run lacked llvm-objcopy on PATH, then passed with the installed
  tool on PATH. Original raw failures remain preserved.
- Current QEMU product, accept, system-fault, Shell smoke, memory-transfer,
  DMA Ruin, Forced and short hardware fence pass at both 4 and 8 Harts: 16 runs,
  all outer raw 0 and independent Root judge raw 0. Each run has one real entry,
  one successful embedded scene, successful capture, full task exit and matching
  physical frame counts: 85 to 85 at 4 Harts, 153 to 153 at 8 Harts. Shell smoke
  includes pipes, pause/resume and cancellation. Existing device and memory
  drivers retain their scenario assertions and resource-cleanup checks.
  The first product scene passed guest assertions and frame reclamation but the
  capture status remained pending. The worker repaired two status redirections;
  Root separately tested successful capture and tee failure, retaining strict
  rejection of a failed capture. The repaired product run is raw 0.
- Nu 0.110 requires a wrapped command for runner flag passthrough. The wrapper
  accepts unknown runner flags and strips exactly one leading CLI separator;
  remaining values/order, timeouts, assertions and error handling are retained.
  Memory-transfer uses its existing Source/trace layout, archived after each run.
- The initial 8-Hart accept failed twice: an absent operator path produced
  Unjudged instead of Unknown. The client code was byte-identical to a297;
  an isolated unmodified baseline's guest probes passed (its old capture status
  still made its outer run fail). Review identified an expired-retry admission
  window. The client now samples remaining budget once per attempt; after a
  genuine UNKNOWN it does not issue a retry whose captured budget is already
  POLL. This intentionally changes that expired-before-call edge result to the
  existing Unknown result. Actual calls' transport errors still produce
  Unjudged; first-POLL, positive-budget failures, resolve and Denied behavior are
  separately tested with the real public IPC exchange/state fixture. Wire values,
  ABI, authority, budgets and probe assertions are unchanged. Independent review
  rejected an intermediate double-clock implementation before publication.
- Root compares locks by (name, version, source): root removes only the old path
  client, adds three provider packages and updates programs' dependency list.
  The image lock adds only the support dev-dependency graph. No unrelated
  versions/checksums are changed. Kernel remains clean at the fixed revision.
- Independent reviewer accepted the provider transformations, including fixture,
  account, identity, device and daemon ownership. This is scoped acceptance.

These reclamation results belong to the embedded scenes above. Ordinary boot
does not emit the testing-only frame record and is not claimed as a separate
reclamation proof. The historical missing-initrd health sentinel is not made
green; the old full Kernel matrix and long 1024-generation scenarios were not
rerun for this slice. Image-builder publication and pipe migration remain separate
slices; Stage 3/4 and the overall migration are not declared complete here.
