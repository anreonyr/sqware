# PCI / IOMMU public contract extraction

This delivery consumes published PCI declarations/API and IOMMU declarations/API/hold.
It is a prerequisite within stage 3, not a complete Driver Unit migration.

## Versions and ownership

- Source base: `1854d607648ceacfa1d998004478c47c3793700c` (PR #44).
- Source branch: `codex/shuyra-device-contracts-extraction`.
- PCI: `https://github.com/shuyra-arche/pci.git`, revision
  `6e3ca09ffc61610017746f74a5f7df0be2ba2dae`.
- IOMMU: `https://github.com/shuyra-arche/iommu.git`, revision
  `5d2038033efcbd55de77534b35e611729d4bd9bd`.

Each provider starts from a reviewed source snapshot and records provenance.
The 34 production Rust files and five package manifests match the Source base
byte for byte. The provider workspaces contain only their owned packages.
The actual declaration `PROGRAM` values move with those packages.
Runtime implementations, clients, `prog-pci`/`prog-iommu` binaries, and the
composition image target descriptors remain in Source. Neither provider claims
to contain the complete Driver Unit.

Source removes the five old package directories and consumes the public packages
through nine remaining direct consumer manifests, with 25 fixed Git dependency
edges. The now-unused retention test dependency on `iommu-hold` is removed.
Source does not retain a second copy of any migrated implementation.

## Tests and locks

Fourteen existing tests move to real provider owners:

| Source assertion owner | Provider owner | Cases |
| --- | --- | ---: |
| PCI API protocol | PCI API tests | 3 |
| IOMMU hold range | IOMMU hold tests | 1 |
| IOMMU pages provider wire | IOMMU API tests | 4 |
| Retention ordering | IOMMU hold tests | 6 |

The three retention ordering fixtures move with their tests. The provider-wire
test adapts imports and the page-size reference; the other moved files retain
their bytes. Source keeps private runtime/queue and Hub retention assertions.
Its IOMMU pages suite changes from 52 to 48 tests and retention from 45 to 39.
The provider suites contain 3 PCI API, 11 IOMMU API, and 7 IOMMU hold tests,
including added IOMMU protocol golden tests.

Of 51 tracked lockfiles, ten change and 41 remain byte-identical. Nine changed
locks add the appropriate Git source fields; the retention lock removes its
unused hold package and dependency edge. Other versions, checksums and unrelated
dependencies do not change. Cross-repository release checks use GitHub sources,
not local overrides. Public clean checkouts and an actual Git consumer were
built before Source was switched to the published revisions.

## Validation of the frozen Source

Evidence is under `/home/anreonyr/Develop/shuyra/evidence/pr22/consumer`.
The code freeze contains 1721 existing build-input files. Its manifest SHA-256 is
`1dbea6ca262d9d1df0488f82a1ecbd93aac351fd7a600d1036c6d9a7a4de8b89`:
hash the manifest bytes including the trailing newline. Documentation is excluded
from these frozen build inputs. Before this document, the Source change contains
46 deleted paths and 23 modified paths. The migration and test-assertion maps
record every removed file and its provider destination.

- Twelve affected host suites pass 234 tests in each of locked and locked/offline
  modes. Image-library tests pass 25 in each mode; the image CLI builds in both.
- The RISC-V workspace check passes in both modes.
- Four normal builds cover debug/release and locked/locked-offline. Each captures
  one Kernel and 89 program binaries immediately after its own Cargo invocation.
  The 360 records form 180 matching canonical artifact rows. Normal release uses
  optimization level 3 with debug assertions disabled.
- Every canonical ELF passes exact defined-symbol checks: one `_start`, one
  `__rustc::rust_begin_unwind`, and at most one `__rustc::__rust_alloc`.
  Four exclusive Kernel copies were captured before the first runtime test.
- The two selected Kernel health tests, external retention ordering and Accord
  retention authorization, each pass on four and eight Harts. Each invocation
  reports one passing selected test and 121 filtered tests; verbose QEMU arguments
  confirm the requested SMP count.
- Eight fresh guests pass: product terminal sessions on four/eight Harts, accept
  terminal session, IOMMU product fault, and four DMA confirmation failures
  (invalidate/reset/release/reference). All raw exits are zero; capture/feed
  status and the independent scene judge are checked separately.

The guest judge checks natural halt, zero exit reasons and unchanged frame counts
(85 for four-Hart normal scenes, 153 for eight-Hart product, 87 for DMA).
The fault scene contains both complete ordered stop/isolate/release chains,
including PCI bus-master off, actual device reset and queue shutdown, IOMMU queue
shutdown, final frame release, detach and auxiliary retirement. DMA failure and
injection removal retain two references. The reference case follows the observed
sender and handle `112` through rejection, retained ownership and release to one
reference before final IOMMU release; it does not assume a fixed allocator handle.

## Failed attempts and limits

Provider testing exposed an incorrect test expectation about redispatch between
same-sized Step frames. The expectation was corrected without production changes;
the original failing exit is retained. Consumer DeepSeek calls encountered
transport and capacity failures. The worker checkpoint is sealed; Root performed
the final build/runtime acceptance. This does not turn the failed worker calls
into successful calls.

Root's acceptance aggregation initially rejected a duplicate `profile` field,
and its health-name matcher initially omitted the module prefix. Both harness
errors were corrected against preserved successful build/runtime records.
No Kernel copy or completed guest log was overwritten to fix these checks.

This delivery does not claim the complete Kernel health suite, an initrd sentinel
pass, a new 1024-round long run, or protective termination as proof of DMA drain.
Full Kernel/System runtime and image-builder extraction, pipe migration, the final
Integration revision/submodule matrix, and stage-4 acceptance remain outstanding.
The DeepSeek coordinator is not established before that acceptance.
