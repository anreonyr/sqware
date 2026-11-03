# Stage 3: Kernel public provider cutover

Status: Source consumer cutover preparation. Kernel public provider:
`https://github.com/shuyra-arche/kernel.git` at
`3aa0a155f825fbf377fc8d04009472ca67cbbf8c`, gitlink at `components/kernel`.
Source base: `672d8d2deafc8067c94b6af340a5f89c6a765c9c` (PR 45).

Kernel is a boot/runtime mechanism, not a Runtime Unit. There is no UnitFile,
server wrapper, Source proxy crate, or Cargo path dependency on a local Kernel
checkout. `components/kernel` is a fixed public gitlink.

## Ownership

- Kernel implementation, health/host tests, fixtures, and its own locks live in
  the public provider.
- Programs, composition, image builder, Shell, drivers, scene feeds, harness,
  firmware, alloc-probe, and Lisp remain in Source.
- Source owns the image build and Integration harness. The provider does not
  bundle either.
- The old tracked `kernel/` tree is removed; historical docs under `docs/`
  that mention old `kernel/...` paths are baseline/non-acceptance references,
  not live commands.

## Cargo commands

Kernel is not a Source workspace member. `programs` and `crates/alloc-probe`
consume the public provider by full Git revision; both use the same public
SHA. The Source workspace excludes `components/kernel`.

Build or run Kernel explicitly:

```sh
cargo image product release
SHUYRA_INITRD=/absolute/path/to/initrd.img cargo run \
  --manifest-path components/kernel/Cargo.toml --release
```

Named alias, same explicit manifest:

```sh
SHUYRA_INITRD=/absolute/path/to/initrd.img cargo shuyra --release
```

Plain `cargo run` no longer selects Kernel. The image builder prints the actual
`ok <absolute-path>`; use that value. No image is copied next to the Kernel
ELF and no image is built inside Kernel.

## initrd selector

`scripts/qemu-args.nu` exports `chosen-initrd`. It selects `SHUYRA_INITRD` when
set and requires that file to exist; otherwise it uses the ELF-adjacent
`initrd.img` if present. The normal runner validates the chosen file before
QEMU, normalizes it absolute, and passes the same value through its trace-cwd
change. `--board-only` never selects or adds an initrd; qtest injects the
Source image path itself.

## qtest

Call from the Source root:

```sh
nu scripts/qtest.nu
nu scripts/qtest.nu --package kernel
nu scripts/qtest.nu --package kernel --scene product
```

Empty `--package` selects `kernel`; a non-kernel package keeps Source workspace
meaning. Only the actual `cargo qtest` invocation runs with cwd
`components/kernel`; `cargo image`, feed scripts, traces, hold/control files,
and judges stay absolute from Source. The installed `cargo-qemu-test` rejects
`--manifest-path`, so the adapter uses a scoped cwd instead. No second runner
or harness protocol is introduced.

## Proof distinction

- **Owned host**: Source test suites and alloc-probe on x86_64. These are
  type/byte/ledger checks, not Kernel hardware proof.
- **Native health**: provider `embedded-test` under QEMU through the Source
  adapter; selected retention tests and full health retain the known missing
  initrd `scene` sentinel. Not all-green.
- **Normal boot**: one documented `cargo run`/`cargo shuyra` boot with a real
  Source-built initrd, natural halt and zero exit. Kernel's `verdict()` frame
  accounting is test-only, so a normal non-testing boot does not emit
  `memory: frame_allocations`; this is recorded as a retained limit, not
  invented proof.

## Final validation evidence

- Source host: 11 suites, 228 passed each locked/offline; the six
  provider-owned Kernel DMA host cases make the prior 234 each.
- Image host: 25 passed each mode; image CLI builds and root RV checks pass.
- alloc-probe five aliases, each mode: 10 / 6 / 14 / 11 / 4.
- Programs 89 artifacts × debug/release × locked/offline plus four Kernel
  copies: 360 records / 180 canonical; exact `_start`/panic/allocator symbols.
- Eight guest scenes pass the pinned judge; Fresh4 and Race4/8 pass their
  existing drivers; the short hold/socket fence reports a real CQ fault and
  frames 85→85.
- Final health selected runs: 1 passed / 121 filtered. Full health: 121
  passed / 1 failed, only the missing-initrd `scene`, raw 101. Not all-green.
- Normal boot reached natural halt and zero exit. Its test-only
  `memory: frame_allocations` line is absent; the helper treats that as a
  failed overbroad normal assertion (helper exit 1). Physical reclamation is
  proven only by real embedded scenes, not by this normal console.

The only script delta after the prior runtime freeze is the conditional Kernel
SHA read: non-kernel packages no longer read `components/kernel`. Kernel
default/explicit invocation, scene flags, timeouts, and assertions are
unchanged. Prior runtime logs predate this guard and are not claimed to have
run it; a scoped fake-tool check validates the cwd and no-git behavior.
Historical `scripts/bench-map-index.py` Kernel paths remain stale baseline
references, not live commands or proof.

## Limits

Stages 3–4 are not complete. System runtime, image-builder publication, pipe,
and the coordinator remain pending. No full-scene, 1024-run, hardware, or
all-green claim is made. No merge, archive, branch deletion, or cache cleanup
is part of this cutover.
