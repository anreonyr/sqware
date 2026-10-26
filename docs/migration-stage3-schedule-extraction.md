# Stage 3: schedule extraction consumer migration

Provider: `https://github.com/shuyra-arche/schedule.git` at
`a806630061a1d939f31ad8c683d544d00a385854` (public default
`codex/source-snapshot`).

Public source: `https://github.com/anreonyr/sqware.git` at
`378d4d826fff70851dfcf08ace8cded4a55d0d2b` on
`codex/shuyra-execution-extraction` (PR35). This migration was performed on
local branch `codex/shuyra-schedule-extraction`; publication status is stated
at validation time below.

## Mapping

Provider: 13 files = 9 production Rust files + 1 root manifest + 3 owner files
(the test `.gitignore`, test `Cargo.toml`, and test `lib.rs`). The provider has
37 tests = the original 36 + the migrated shape test.

Consumers: 10 manifests now point `schedule` at the public Git provider and the
full SHA above: `programs/Cargo.toml` and
`programs/tests/{bootstrap-carrier,control-instance,control-query,daemon-control,
guard-deployment,iommu-pages,retirement-boundary,shutdown-policy,startup-authority}/Cargo.toml`.
`iommu-pages`, `retirement-boundary`, and `shutdown-policy` keep it under
`[dev-dependencies]`; the others keep their existing dependency tables.

The root workspace no longer lists `crates/schedule`; all 13 old tracked files
were deleted. The only source-test edit is the removal of
`schedule_mechanism_has_no_domain_dependencies` from
`programs/tests/system-shape/lib.rs`: system-shape goes from 22 to 21 tests.
`References` and the recursive `references` helper remain for the other tests.

Locks: only `Cargo.lock` and the 9 suite locks changed. Each gained exactly one
`schedule` source line (path to Git); all other complete package records are
unchanged. `programs/tests/system-shape/Cargo.lock` and every other lock are
byte-identical to the frozen source. For this schedule consumer dependency there
is no implementation copy, local patch, remap, or cache projection. The unrelated
pre-existing alloc-probe vendor patch was not touched.

## Verification

- 9 suites: locked and offline actual passes 12/39/10/14/15/53/11/8/6 = 168.
- system-shape: locked and offline actual 21 passes.
- RV64: workspace check, kernel 1 + programs 89, debug and release; 90 unique
  ELFs (180 profile rows) with hashes. Symbol audit: one `_start`, at most one
  allocator, one `rust_begin_unwind` per ELF; 7 debug and 10 release programs
  need no allocator.
- QEMU 5 scenes: product terminal 4/8 hart, product shell smoke 4 hart, accept
  terminal 4 hart, system-fault 4 hart. All raw exits 0; HART 4/8, unique entry
  task 1, reason 0 completion, natural halt, frames 85 to 85 or 153 to 153,
  capture 0, four fed scripts ok. No protective termination used as proof.

## Actual Cargo JSON supplement

The four normal builds were repeated with `--message-format=json`, `--locked
--offline`, and the same target/profile flags, under
`evidence/pr13/consumer/builds/`:

- `cargo-json-kernel-debug.json` (exit 0): kernel `shuyra` 1 executable.
- `cargo-json-programs-debug.json` (exit 0): programs 89 executables.
- `cargo-json-kernel-release.json` (exit 0): kernel `shuyra` 1 executable.
- `cargo-json-programs-release.json` (exit 0): programs 89 executables.

The actual artifact table `artifacts/elf-artifacts.tsv` has one row per profile
artifact (180 rows, 90 unique ELFs) with the real `package_id`, `target.src_path`,
profile, executable path, ELF header, and SHA-256. Every hash matches the previous
table and the saved kernel copies, so the existing `nm` symbol records remain
valid. The schedule artifact appears as
`git+https://github.com/shuyra-arche/schedule.git?rev=a806630061a1d939f31ad8c683d544d00a385854#schedule@0.1.0`
with `src_path` under
`~/.cargo/git/checkouts/schedule-590e521f0fbff70c/a806630/src/lib.rs`.

## Not verified

Stage 4 full matrix, remaining stage 3 work, and the additional IOMMU/1024
scenarios are not part of this round. No coordinator was created. At validation
time, this branch was uncommitted and unpublished; root publication supersedes
that snapshot.

Author and license facts are recorded in the provider's `PROVENANCE.md`.
