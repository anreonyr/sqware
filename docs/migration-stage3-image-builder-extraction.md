# Image-builder extraction

Source base is `aaed018fb2c6b9088f2d06fb6a8d5e23f984a21a` (System PR47).
The public provider is `https://github.com/shuyra-arche/image-builder.git` at
`17f294bae58ef5526a18fd6a88a88cc41850d4a6`, fixed by both the Cargo dependency
and `components/image-builder`. The package/library is `image-builder` /
`image_builder`; Source retains the `image` dependency alias. Kernel remains
`3aa0a155f825fbf377fc8d04009472ca67cbbf8c`, System remains
`5a792ff2d39901a970925b1a6d63206adf6c16d8`, API remains
`9e5d63ca536a176583cf1b0054b4ecf7ddd88257`.
This is a public Git-SHA release, not a crates.io release.

The provider accepts explicit declarations, selected Deployment and ImageBuild
coordinates. Image identity, Cargo package, binary, features, target, profile and
default-features stay separate. Cargo metadata and compiler-artifact executable
paths remain authoritative. Source owns composition, scene/profile selection,
build root/output policy and the CLI. The cargo image alias is unchanged.
Original implementation and provider-owned fixture files are removed from
`crates/image`; there is no fallback, patch override or Source backlink.

Public image_inventory and startup_order reuse the original planning algorithms;
build_with calls those same functions. Planning does not replace coordinate
uniqueness validation or Cargo resolution. Capsule Error/Region/Plan and
BuildGroup now each have their own file. Capsule bytes, error variants, padding,
zero-fill, grouping, requested profile/features and CLI output remain unchanged.
One corrected coordinate case fans a single Cargo executable out to all logical
image names sharing that package/binary coordinate; aliases are not rejected.

Test ownership preserves all 25 original names and assertions: nine pure cases
and four real-artifact cases belong to the provider; nine actual-composition
cases and three cross-component catalog cases belong to the Source CLI. The
additional real alias resolution/packing case brings the provider to 14 and
Source to 12. Provider normal and dev graphs have no Source/composition dependency.
Provenance, hashes and split ownership are recorded in the provider.

## Actual verification

- Root ran provider 14 tests, then a fresh public checkout repeated all 14 with
  explicit host target, locked/offline and nested RISC-V Cargo builds. Real
  llvm-objcopy is on PATH. Cache warming is allowed; cold-cache offline resolution
  is not claimed. Provider metadata has 21 packages and no Source path edge.
- Source CLI 12 tests pass, locked/offline. Normal Cargo resolution generated
  both locks; existing package tuples/versions/revisions/checksums remain intact.
  The consumer lock replaces only the path image package and adds the published
  builder plus catalog-test support graph. Root workspace lock is unchanged.
- Real cargo image product/accept release builds pass. Root records the actual
  Cargo coordinates, executable paths, preserved ELF copies and hashes: 17
  product images and 39 accept images, with preserved initrd files.
- Current product and accept QEMU runs pass at both 4 and 8 Harts: four outer
  raw exits 0 and independent Root judge 0. Completion belongs to the unique
  actual entry task; full task exit, successful capture and physical frame
  restoration are required: 85 to 85 and 153 to 153 respectively.
- Two changed probe binaries also build in RISC-V debug/release, locked/offline;
  their four ELF artifacts are preserved with unique startup/panic and no
  duplicate allocator. Other runtime binaries were not changed by this slice.

## Permanent-negative probe transport

The first image-builder 8-Hart accept run failed two existing negative assertions
with Unjudged instead of Unknown. Before any probe edit, all 39 accept ELF hashes
matched the previously accepted System slice. The image extraction did not
change those runtime binaries. The failure log does not identify the underlying
CallFail variant; a timeout explanation is an inference, not a measured fact.

The retired-operation and post-unbind probes had used publication-wait lookups
for permanently absent paths. An admitted positive-budget RPC may legitimately
fail and return Unjudged; the SDK's expired-before-call guard does not promise
otherwise. The two probes now issue one typed Relative RPC on the same existing
session, base and path with the original 1000ms maximum. They require the exact
published UNKNOWN wire status. A transport failure panics with its actual Debug
variant; no error is remapped or accepted. All eight retired paths, the unbound
tile, positive client calls, role/owner/independent-session assertions and the
retained mapping read after unbind remain checked. System/API versions, budgets,
authority, transport lifecycle and reclaim checks are unchanged.

Coverage change is explicit: these runtime negatives now pin the server reply
directly, rather than end-to-end retry_missing=true exhaustion-to-Unknown mapping
inside Face::reference_query. The existing 15 real-IPC shared retry/guard host
tests and positive runtime Face calls remain, but they do not replace that exact
negative Face wiring coverage. Independent DeepSeek review accepted this
probe-only seam change; no identical-coverage or scheduler-determinism claim is
made. The failed run is preserved alongside the new four passing runs.

## Retained limits

Output fs::write is non-atomic; Err means the path is not valid. Callers serialize
one work/output pair, and group-N directories are rebuildable Cargo cache. The
existing minimum non-entry ELF checks, crafted-ELF bound and fixed page shift are
retained. The repository keeps nightly-2026-09-04 and asserts no stable/MSRV or
new license grant. Ordinary boot does not prove testing-only frame reclamation;
the historical missing-initrd health failure, full Kernel matrix and long 1024
scenarios are not newly claimed. Pipe migration and Stage 3/4 completion remain
open. Evidence is under `/workspace/evidence/image-builder`; a wholly fresh
DeepSeek namer/worker/reviewer group produced and reviewed this slice.
