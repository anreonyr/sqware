# Guard death scene

## Scope

This scene proves that a deployed PCI, IOMMU, or Recovery static domain is
guarded before a real in-flight block request is submitted, and that killing the
protected domain main task after submission stops the machine before reference
cleanup. It does not claim that DMA was drained: the successful case deliberately
retains the block request and physical frame references.

The three deployments are `iommu-guard-pci`, `iommu-guard-iommu`, and
`iommu-guard-recovery`. They share entry image `iommu-guard`, start
`hub`, `pci`, `iommu`, `recovery`, `device-ctl-guard`, and select exactly one
guarded name. The shared entry is the scene Root; the controller image is
`device-ctl-guard`.

## Root and trigger

`programs/src/harness/probe/device_guard.rs` takes the bootstrap, opens the
`ns16550a` resource from `Machine.devices` through `Accounts.token` and
`Dock::open`, then installs the production resources. The included
`driver/uart/dev/uart.rs` supplies only `drain`; no production UART branch is
added.

Each Root tick has one `prepare`, one budget-1 subplan that advances the real
`app::schedule::frame()` once, and one `trigger` poll. The app-frame cursor lives
in `Turns`, so the production frame is advanced exactly as the normal supervisor
advances it while startup remains free to progress. `prepare` reads the private
`device-guard/request` Hole and accepts only the real controller sender and the
single byte `[1]`.

The trigger step observes the one held Control row, requires `guard_held`, the
actual `task`, and the actual `task_team`, and prints:

```text
guard: armed target=<name> task=<id> team=<id>
guard: submitted controller=<id>
guard: ready target=<name> task=<id> team=<id>
```

On a valid `G` byte, it re-reads the same Control row in that System frame,
requires the same target, task, team, and a live task, then prints
`guard: trigger ...` and calls `env::unit::slay_task(task)`. It never calls
Control Ruin or Unguard. If Slay returns, the Root logs that fact and parks; it
does not report completion or run cleanup.

The controller reuses the Ruin submission path: real in-flight block request,
`want == 3`, then the private Hole. It keeps the block page, view, references,
command channel, and launch channel open and never closes or releases them.

## Kernel instrumentation

Under `critical-scene`, `member_dying` logs the actual dying task, its own Team,
and the protected ancestor Team after `ExitPolicy::trip()` actually returns true:

```text
[critical] trip task=<id> task_team=<id> team=<id>
```

The existing guarded-exit, peer-quiescence, and reference-cleanup records are
unchanged. The trip record contains identities only; it does not name hardware or
change the protection decision.

## Evidence

The host driver `programs/tests/device-recovery/guard.py` waits for NBD
`phase=held`, the in-flight doorbell, and `guard: ready`, confirms that the read
was not resumed or completed, copies the held console and NBD events, sends one
`G`, then records trigger, guarded exit, and serial EOF. It writes
`<capture>.feed.json`, `<capture>.held-console`, `<capture>.held-events`, and
`<capture>.block-events` before `qtest.nu` stops the background NBD server.
`DMA_HOLD_SECONDS` is 60, longer than the VM timeout.

`programs/tests/device-recovery/guard_check.py` independently requires:

- raw qtest exit 101, a real timeout, and the `embedded::tests::scene` failure;
- one armed, ready, and trigger record with the requested target and identical
  task/team identities;
- one kernel trip whose task and protected Team match the trigger, followed by
  the guarded exit and exactly `harts - 1` peer quiescence receipts;
- no target reference cleanup, no normal shutdown, no panic, and no re-mint or
  reactivation of the target after the trigger;
- no pre-trigger invalidation, bus-master-off, release, detach, or close;
- a held NBD request in both the pre-trigger snapshot and the final record, with
  no resume, completion, or resume receipt.

The fixture `programs/tests/device-recovery/fixtures/guard.py` and cases in
`test_guard.py` cover missing held, wrong target, missing quiescence, wrong hart
count, target cleanup, normal return, missing trip, out-of-order trigger,
identity mismatch, pre-trigger release, and retention failures.

## Commands

Run the commands separately, one QEMU at a time. The runner is expected to exit
101; the independent verifier decides acceptance.

```sh
CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true QEMU_SMP=4 \
TRACE_OUT="$PWD/target/driver-composition/guard/pci-4" \
nu scripts/qtest.nu --package kernel --scene iommu-guard-pci -- \
  --features semihosting,critical-scene --timeout 30 -- scene
# qtest.nu already invokes guard_check.py with the raw runner code. To inspect
# the same evidence manually (101 is the expected raw timeout):
python3 programs/tests/device-recovery/guard_check.py \
  target/driver-composition/guard/pci-4/scene-iommu-guard-pci-console.log \
  target/driver-composition/guard/pci-4/scene-iommu-guard-pci-diagnose.jsonl \
  target/driver-composition/guard/pci-4/scene-iommu-guard-pci-feed.log \
  pci 4 101 \
  target/driver-composition/guard/pci-4/scene-iommu-guard-pci-qtest.log
```

Repeat with target `iommu`, `recovery`, `QEMU_SMP=8`, and the corresponding
scene. `qtest.nu` uses this independent verifier for the new scenes only; it
keeps the raw runner code, does not map an arbitrary 101 to zero, and leaves the
old scene criteria untouched. The NBD event copy is
`scene-<scene>-block-events` in the same trace directory.

## Verification performed by this worker

No QEMU was run. The worker builds were:

```text
cargo check --workspace --offline                                  pass
cargo check -p kernel --features critical-scene --offline          pass
cargo check --tests -p kernel --features semihosting,critical-scene \
  --target riscv64gc-unknown-none-elf --release --offline          pass
cargo image iommu-guard-pci release                                pass
cargo image iommu-guard-iommu release                              pass
cargo image iommu-guard-recovery release                           pass
cargo test --manifest-path crates/image/Cargo.toml \
  --target x86_64-unknown-linux-gnu --offline                      18 passed
(cd programs/tests/device-recovery && PYTHONDONTWRITEBYTECODE=1 \
  python3 -m unittest test_guard)                                  14 passed
```

The native three-target 4/8-hart runs and the raw-101 verifier runs remain for
the Root thread. This document does not claim A7 completion, DMA drainage, or a
successful native halt.
