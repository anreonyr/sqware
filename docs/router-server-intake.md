# Router server intake

Source now registers the published Router Unit through Driver and builds its
server from the selected nested Router repository. The previous Source Router
runtime and local declaration are removed. Router retains its original slot
between lodger and uart in both global lists; Deployment files are unchanged.

## Fixed public inputs

| Input | Commit |
| --- | --- |
| Source parent, Router client intake | `c975cec91273261509c8f9f715643830c2fed5c5` |
| Driver aggregate | `b82a20f36684c13808d89b0f1a5b4ce2796c2dbf` |
| Router server and declaration | `8a9416277539aefbc759c4f6519e1e8d93d63be3` |
| Router API | `94fec223f3c5df6c99a49527a65d016d88de3af4` |
| Router client | `aa3033a7bd25b5d4199a50a6c00538475b62ef4c` |
| Server's Driver shared support | `e75b1e232f828666956fb7e02c61a7d84f13eb66` |
| Existing Source driver-support | `8e6de5a77aa52184bb6f7fbbcbc961f779e94629` |
| Hub Unit | `daa23b2611fee0c503bc6bd31a16121d97698ad6` |

Router and Driver were each independently checked from clean public clones
before this consumer intake. The Router runtime has its own locked workspace;
Source's lock file alone is not proof of that runtime's dependency closure.
Both closures were checked separately. Source and image-adapter locks change
only the Driver cluster identity and add the fixed Router declaration.
Existing Git URLs are retained, including repositories subsequently renamed.
Cargo source equivalence between old and renamed URLs is not claimed.

`IMAGE_BUILDS` delegates Router fields to its real `BUILD`, with the manifest
prefix `components/driver/units/router/server/Cargo.toml`, package
`router-server`, and binary `router`. There is no `prog-router` fallback.
Source retains `fdt` for the memory-transfer probe and retains the shared modules
used by other local servers. This change does not migrate RTC or adopt kernel-api.

## Validation on 2026-11-04

- Root compared 35 Router runtime items, startup, declaration and four handoff
  cases with the accepted provider before removing exactly 17 old files.
- Source registry and image-list ASTs preserve their original order. The
  handoff fixtures and assertions differ only in the helper source path.
  All 92 pre-existing system-shape assertions remain across the same seven cases.
- Locked online/offline metadata and dependency identities passed for Source,
  the separate Router workspace, and the image adapter. Composition passed its
  RISC-V check.
- 31 distinct host cases passed: router-handoff 4, device-api 3,
  interface-marks 5, system-shape 7, image composition 9, catalog 3.
- An actual isolated-target release RISC-V Router ELF passed artifact,
  entry/load-segment, symbol uniqueness, unresolved-symbol and gzip-roundtrip
  checks. Its 317,256-byte SHA-256 is
  `7f85c267b2f18f5535e3d5b8018a33bc15318b6ad508b3cc24cab4080ee27df8`.
  The image builder's selected Router ELF has the same hash.
- After the user's instruction to run only once at SMP8, exactly one final
  `accept` SMP8 run passed: raw exit 0, root exit `0x0` / `accept: done`, one
  natural halt, no panic, frame allocations 153 before and 153 after. Source
  inputs remained unchanged during the run. No further whole-machine run was
  performed. The initrd SHA-256 is
  `ddac50c9396973cbf182e43b5e3f246ea2d9696373dd98bbfc4ed4f708e57be3`.

Earlier failures remain part of the evidence. The initial image-composition
test failed because its generic local-program assertion did not yet distinguish
Router; the same DeepSeek worker added the real Router BUILD assertions without
removing the fallback checks. An initial offline metadata command lacked cached
`syn 3.0.9`; locked online fetch followed by locked offline verification passed.
Before the SMP8-only instruction, one SMP4 accept run completed with 85/85 frame
recovery, but Root's unchanged-input guard then failed because the standalone
image-adapter lock had updated during image construction. SMP8 had not started.
That lock change was independently verified before the final single SMP8 run.

This is not a full historical scenario-matrix regression. Source debug ELF and
other current whole-machine scenes were not tested. Provider-level Router debug
and release checks are separate evidence, not additional Source runtime runs.

## Authorship and evidence

This PR used a new genuine `opencode-go/deepseek-v4.1-flash` group:

- namer: `session-0691c139-b190-47a6-bb44-390807f6be50`
- worker: `session-26370b56-dfbb-412b-ab56-4f0d7b7f9e7c`
- reviewer: `session-f52576bb-b3b8-4355-851c-5ac31cf76150`

Worker authored the integration and test edits. Root independently ran checks,
maintained locks/gitlinks/docs and mechanically removed the verified old files
while worker was idle. No model shell commands were forwarded. Execution
records, raw failures, role audits and reviewer results are under
`/workspace/evidence/source-router-intake` in the execution environment and the
subsequent safe handoff archive. No merge or history rewrite is part of intake.
