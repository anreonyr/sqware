# Guard deployment wiring

## Selection

`crates/unit/src/deployment.rs` carries `Deployment.guarded`. Device deployments
that start the three trusted domains explicitly declare
`guarded: &["pci", "iommu", "virtio-blk"]`; other deployments declare an empty
selection. Hardware names appear only in these deployment declarations.
The current Driver naming and final acceptance are recorded in
[driver-device-boundary.md](driver-device-boundary.md); historical native evidence
retains the unit names used by its own source checkpoint.

`Deployment::guarded(units)` validates the declaration through
`crates/unit/src/guard_selection.rs`: names must be unique, present in that
deployment's own `start`, and present in the caller-provided Unit declaration
slice. The validator contains no hardware name. `crates/image` checks every
deployment through the same explicit-slice method.

## Boot authority

`programs/src/system/app/install.rs` takes `Name::Call(Call::Guard)` from
`boot.accounts` before `boot.accounts` is moved into `Supplies::new`.
`programs/src/system/app/guard.rs` accepts the authority only when the selection
is empty (returning no token) or when the token is alive, a Nole, `FETCH`-only,
and owned by the running System task. Missing or invalid authority fails
installation; it does not silently degrade to an unguarded deployment. The token
is injected into Control's configuration; `Supplies` never grants it to drivers.

## Control obligation

`programs/src/system/control/unit/table.rs` stores `Service.guard_held`. It is
set only after a real Kernel Guard success and cleared only after a real Kernel
Unguard success. `attach` and `detach` refuse to replace or clear a slot while
the obligation is held. `Control::guard_required` reads the trusted selection
directly; no second declaration cache or death ledger exists.

## Lifecycle

First embark (`programs/src/system/control/lifecycle/embark.rs`): the spawned
team is Live, and Kernel Guard requires `Debarked`. Control therefore debarks the
whole team and waits for every member to be off-hart, calls Guard, records
`guard_held`, then embarks the team and finally the first held main task. Guard
failure leaves the obligation unset and does not call Unguard. A failure after a
successful arm keeps the obligation; `Active.finish` already routes that job to
Ruin.

Ruin (`programs/src/system/control/lifecycle/ruin.rs` and `schedule.rs`): the
external retirement gate still runs first. `ruin::pre` records the actual slot
and task and moves Control to `Stopping`. The new `guard` step then debarks the
whole team and calls Unguard; Busy keeps the step Pending within the existing
deadline, and any other failure follows the existing Shutdown path without
slaying. Only after Unguard succeeds are namespace/identity installations
retired, and only then does `ruin::run` slay and `ruin::post` oust. `ruin::run`
refuses to destroy a slot whose obligation is still held. `Control::resume`
embarks a paused team without re-arming.

No syscall or Kernel permission was added. Kernel slots 43/44 and their existing
Guard role checks are unchanged.

## Host results

Run from this worktree:

```text
cargo test --manifest-path programs/tests/guard-deployment/Cargo.toml \
  --target x86_64-unknown-linux-gnu --offline
  16 passed

cargo test --manifest-path programs/tests/shutdown-policy/Cargo.toml \
  --target x86_64-unknown-linux-gnu --offline
  8 passed

git diff --check
  pass
```

The earlier guard Host test used a local `launch` helper that mirrored
`lifecycle/embark.rs`; it did not compile the production Embark. That helper is
removed. The current fixture includes the real `lifecycle/embark.rs` and drives
its `pre`/`run`/`ready`/`post` through `schedule::Resources`/`ResMut`, with
minimal fake Control, Roster, and Service boundaries. It compiles the real
`unit/guard.rs`, `unit/table.rs`, `lifecycle/ruin.rs`, and `lifecycle/embark.rs`.
The fixture files are split by fake Kernel, Control, lifecycle, app, and
assembly roles, one data structure per file; the cases remain in `tests.rs`.

The guard Host tests cover: unselected domains never call Guard; Live ->
debark -> Guard -> team resume -> main task start; Guard failure leaves no
obligation and does not start the task; Busy stays Pending until the deadline;
team-resume failure and task-embark failure after arm both keep the obligation
and are released through the real `ruin::release_guard`; external retirement
Pending performs no pause/Unguard/slay; on-hart Busy does not release; Unguard
failure keeps the obligation and blocks destroy; a successful release happens
once and repeated retirement does not repeat it; resume keeps the held
obligation without re-arming Guard. Negative authority tests cover missing,
unavailable, not-alive, non-Nole, wrong-owner, and non-FETCH-only tokens.
Negative selection tests cover unknown, duplicate, and unstarted names. The
shutdown-policy Host tests assert the `detach_unit` Result at every call site;
the probe fixture likewise asserts its detach Result instead of discarding it.

## Not verified here

The native DMA-active three-role scenario (4/8 core member death) and the
independent non-return halt verifier were not run in this round. These Host
tests do not establish A7 completion; the native scenario and the independent
halt verifier remain required before that claim. The known A2
`namespace-installation` host diagnostic (missing debug shim in the other tree)
was not changed or reinterpreted here.

## Root review and deployment checks

The initial writer stopped before the next writer began. Its candidate was
sealed, then based on accepted A2 `17226ba1`; the namespace issuer correction
and its debug shim are now present. The fixture revision is a separate
checkpoint. The original mirrored launch helper has been removed; the new
cases compile the production Embark source. Missing rows still permit repeated
detach without reporting new reclamation, preserving the original policy.

Root independently reran guard-deployment 16, shutdown-policy 8,
namespace-retirement 4 and namespace-installation 4: all passed, with unchanged
safety assertions. The new fixtures have at most one struct/enum per file.
The scoped read-only reviewer (DeepSeek Flash,
session-d7f6de27-3fd1-44cd-81c4-4f610c131e9d) found no confirmed blocker; its
limits included the kernel role and global teardown path. Root additionally
checked real lifecycle error-to-Ruin conversion and the kernel exit hook order:
child-domain doom precedes the parent's reference cleanup. Guard-held failure
therefore cannot silently proceed through the ordinary destroy/supply steps.

On production Guard source `d61ad202`, root's sequential native runs passed:

- Four-hart `iommu-product`, 5.38 s.
- Eight-hart `iommu-ruin`, 3.18 s: real held NBD read, alpha tasks 10/11 reaped
  before Driver 8, final frames 153→153.
- Eight-hart `iommu-forced`, 11.04 s: real held NBD read, alpha tasks 9/10 reaped
  before Driver 7, final frames 153→153.
- Four-hart complete `accept`, 10.65 s, including the fixture's asserted detach
  Result and the ordinary deployment's empty guard selection.

Evidence is under `target/driver-composition/guard-deployment/initial-*/`, with
runner logs and per-scene consoles. These runs confirm normal deployment and
retirement compatibility. Unexpected PCI/IOMMU/Recovery member death during
held DMA and the strict four/eight-hart halt verifier remain required; A7 and
the full refactor are not yet complete.
