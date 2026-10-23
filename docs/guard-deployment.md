# Guard deployment wiring

## Selection

`programs/src/unit/deployment.rs` carries `Deployment.guarded`. The 27 deployment
literals are explicit: the thirteen `iommu-*` scenes whose `start` contains the
three trusted domains declare `guarded: &["pci", "iommu", "recovery"]`; every
other deployment declares `guarded: &[]`. Hardware names appear only in these
deployment declarations.

`Deployment::guarded` validates the declaration through
`programs/src/unit/guard_selection.rs`: names must be unique, present in that
deployment's own `start`, and present in `PROGRAMS`. The validator contains no
hardware name. `crates/image` checks every deployment through the same method.

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
  13 passed

cargo test --manifest-path crates/image/Cargo.toml \
  --target x86_64-unknown-linux-gnu --offline
  15 passed

cargo check --workspace --offline
  pass

git diff --check
  pass
```

The guard Host fixture compiles the real `unit/guard.rs`, `unit/table.rs`, and
`lifecycle/ruin.rs` sources against a controlled fake Kernel. It covers:
unselected domains never call Guard; Live -> debark -> Guard -> team resume ->
main task start; Guard failure leaves no obligation and no Unguard; embark
failure after arm keeps the obligation; external retirement Pending performs no
pause/Unguard/slay; on-hart Busy does not release; Unguard failure keeps the
obligation and blocks destroy; a successful release happens once and repeated
retirement does not repeat it; resume does not re-arm. Negative authority tests
cover missing, unavailable, not-alive, non-Nole, wrong-owner, and non-FETCH-only
tokens. Negative selection tests cover unknown, duplicate, and unstarted names.

## Not verified here

The native DMA-active three-role scenario (4/8 core member death) and the
independent non-return halt verifier are not covered by these Host tests. They
remain the next deployment-level step. The known A2 `namespace-installation`
host diagnostic (missing debug shim in the other tree) was not changed or
reinterpreted here.
