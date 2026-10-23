# Daemon Control and resident process (phase 2 checkpoint)

Base `933793e0` retains the phase 1 Control contract: the run-intent fact,
supervision admission, the public Observe/Recover facets, server/client/namespace
wiring, and host proof. This checkpoint adds the actual single ordinary resident
daemon process and its product deployment.

Root has verified actual RTC death/restart on 4/8 harts, ordinary product
startup/shutdown on 4/8 harts and actual Shell interaction on 8 harts. The ordinary
multi-member target and explicit-stop native races remain to be completed. No
hardware-chain or kernel ABI change is part of this checkpoint.

## Sole mutable fact

Control stores `run_requested: bool` on the existing static `Service` row in
`programs/src/system/control/unit/table.rs`. It defaults to `false`. Observed
`State`/`Slot`, `Ending`, and the actual caller are not copied into new state.

The source caller is the kernel RPC `Incoming.from`; Control resolves it against
the existing row (`named`, `Slot::task`, live state), the deployed
`Control.inputs.program`, and the trusted deployment target group. There is no
caller-slot registry, run enum, task/team shadow book, or recovery manager.

`ControlConfig.supervision: Option<&'static str>` authorizes an existing
`Deployment.targets` group. Install validation
(`programs/src/system/control/supervision.rs`) requires the group to exist, have
nonempty and unique actual start members, exclude the supervisor itself and
guarded targets, include the group in the supervisor's `after:
Dependency::Target(...)` ordering, and be declared by at most one deployed
input. No business name is special-cased in System.

## Intent policy

`programs/src/system/control/lifecycle/intent.rs` is the single policy:

- accepted explicit Mint and Startup Mint set `run_requested = false`;
- accepted normal or startup Embark sets it to `true`;
- accepted explicit Debark or Ruin sets it to `false`;
- recovery Mint, recovery Embark, natural reaper Ruin, closing Ruin, and failure
  rollback retain the existing value.

A stage applies the change only after its own validation and reservation. Wrong
state, Unknown, NotReady, Full, rejected admission, or a failed RPC therefore
does not silently rewrite intent. An explicit Ruin that is merged into, or takes
over, an in-flight job commits its stop at admission after the waiter/job
reservation and takeover validation succeed; the old stage may already have
passed its own intent step, so it cannot be the only writer.

## Observe facet

Public `control/supervision` (grant code 6, own mark and receiver) uses the
typed contract in `programs/src/system/api/src/control/observe/`:

- request: `index` plus return token;
- reply: `status`, echoed `index`, `present`, explicit `end`, target `name`,
  observed lifecycle `state`, and `run_requested`.

The list is the caller's declared supervision target group. `end` is the only
EOF signal; a denial is `status = DENIED` with `end = false`, and an absent row
is `present = false` without pretending to be EOF or `Dead`. Every request
re-resolves the actual caller as the named, live, ready static supervisor.

## Recover facet and lifecycle

Recover is a new legacy opcode (`Wire::Recover(name)`, code 12) on its own
public grant (code 7), reusing `Control::Said` and the existing reply/waiting
machinery. The lifecycle uses the same `Tracked` and the same Mint plan for a
conditional recovery Mint; a successful Mint transitions that exact job to
`Embark { parent: None }`, keeps its callbacks, and resets cursor/deadline once.
Success is not returned until the Embark phase actually completes.

Admission re-checks the actual caller, caller liveness/intent, target membership,
target guard status, target intent, global closing, target `Dead` + `Slot::None`,
and conflicts. Effect rechecks repeat caller scope, caller intent, target intent,
and closing before construction and immediately before `embark_task`; they do
not re-demand `Dead` + `Slot::None` after Mint attached the new service.

## Cancellation and cleanup

An explicit valid Ruin that arrives while the same target has an in-flight
recovery takes over that job/allocation in place. It answers the original
recovery callback(s) `NotReady`, keeps the constructed task/instance/pending
ownership, changes the job to a normal Ruin, and resets its cursor/deadline for
cleanup. No constructed Team is dropped to make cancellation easy.

After Mint moves the Service into `Control.pending`, `execution.instance` is
`None` while `execution.task` still owns the new Team. The recovery rollback
predicate therefore keys on the recovery origin plus the surviving task, not on
`instance.is_some()`. A Ruin failure with a surviving task is not a retryable
job: `Tracked::finish` escalates it to a global shutdown fault. A Ruin failure
without a surviving task simply completes that job. Natural reaping continues to
submit a normal Ruin and retains intent.

## Wiring and client

- `programs/src/system/api/src/control/` owns the granted opcodes, Observe
  frame contract, and mark declarations. Existing codes 1–5, opcodes 1–11, and
  the `Said` ABI are unchanged.
- `programs/src/system/control/endpoint/` receives legacy requests (skipping
  Observe) and serves the typed supervision receiver.
- `programs/src/system/control/namespace.rs` publishes State, Observe
  (`control/supervision`), and Recover as `Permit::Public`; Mint/Embark/Debark/
  Ruin remain `Permit::Identity(Selector::Exact(root principal))`.
- `programs/src/system/client/src/control/` exposes bounded
  `Observe::begin_observe`/`poll_observe` and `Face::begin_recover`/
  `recover_reply` over the existing IPC sender/pending primitives. A denied or
  failed reply is an error, not a successful observation or recovery.

## Daemon process and deployment

`programs/src/service/daemon/` is one ordinary user-mode Resident program. Its
UnitFile declares `supervision = Some("supervised")`, `after` that target plus
the Operator bootstrap, and the existing READY signal. The daemon obtains only
the public `control/supervision` and `control/recover` tokens through the actual
namespace, then checks each token's mark and that both were opened by the same
System host. It holds no legacy State/Mint/Embark/Debark/Ruin face.

The runtime signals READY before its first observation. Initial Denied replies
are retried only inside a fixed startup window; after the first valid observation
a denial ends the process. Observe and Recover use bounded pending/poll budgets
with bounded backoff, explicit EOF resets the enumeration index, and only
`present && Dead && run_requested` requests recovery. Recovery is always
conditional and a result or failure leads back to fresh observation.

The expected Control host is the daemon's actual System parent. The Operator
session peer owns namespace responses, and is a different task. Root review
corrected the endpoint owner check to use the System parent before native runs.
Face discovery shares one startup deadline. Failed recoveries retain their
bounded backoff across fresh observations, and a successful recovery waits one
poll interval before observing again.

`product` and the four `iommu-product` scenes start exactly one daemon and add a
`supervised` target containing only `rtc`. The existing scene target and start
list include the daemon, so the existing after-target dependency stops it before
`rtc` on shutdown. Guarded PCI/IOMMU/recovery targets are never supervised. Other
scenes are unchanged, and System has no daemon-name condition.

## Verification

Host checks used for this checkpoint (all offline):

```text
cargo check --offline --workspace
cargo check --manifest-path crates/image/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/daemon-control/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/daemon-runtime/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-query/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-rpc/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-instance/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-lifetime/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/guard-deployment/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/retirement-boundary/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/shutdown-policy/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/system-shape/Cargo.toml --target x86_64-unknown-linux-gnu --offline
```

`programs/tests/daemon-control` compiles production table, intent, admission,
effect recheck, actual `Tracked::finish`, transition, takeover, and
install-supervision logic directly. It covers accepted/rejected intent, explicit
Ruin merge/takeover committing stop at admission before and after the old Ruin
`pre`, natural cleanup retaining intent, caller revocation, global closing,
guarded/out-of-scope/dynamic-copy denial, takeover before build and after Mint
moved the Service to pending, the retirement-complete gate, explicit
end/denial shapes, and the recovery rollback obligation with `instance = None`
and a surviving task.

Root independently reran 137 phase 1 host checks across daemon-control (12),
control-api (10), control-query (9), control-rpc (9), control-instance (33),
control-lifetime (8), guard-deployment (16), retirement-boundary (11),
shutdown-policy (8), and system-shape (21).

Phase 2 host checks ran offline: workspace `cargo check`, image `cargo check` and
tests (16 unit + 3 catalog), daemon-runtime (5), daemon-control (13),
system-shape (22), shutdown-policy (8), guard-deployment (16),
retirement-boundary (11), control-api (10), control-query (9), control-rpc (9),
control-instance (33), and control-lifetime (8). daemon-runtime compiles the
actual decision/startup helper; daemon-control now also compiles the actual
`closing_service_names` ordering. Root independently reran 83 focused host/image
checks and the actual daemon build, then repeated affected runtime/Control/shape
checks after the owner/deadline/backoff corrections.

Root's first debug-assertion product run returned inner qtest 0 and exited
System task 10 successfully with frames 87 -> 87, but the outer runner returned
1 because its completion check assumed task 1. The runner now requires exactly
one actual Kernel boot-entry identity and a matching successful exit. It retains
the failure on missing, duplicate, wrong-task, or unsuccessful exit evidence.

After that correction, strict product 4/8-hart runs passed in 5.48s/5.58s with
frames 87 -> 87 / 155 -> 155. The actual Shell smoke on 8 harts passed in 27.01s
with frames 155 -> 155. Each run recorded one daemon activation, actual boot
entry task 10, its matching successful System exit, and normal halt. Evidence:
`target/driver-composition/daemon/phase2-{product-final-4,product-final-8,shell-final-8}/`.
These are ordinary-runtime checks; they do not prove restart recovery.

## Actual RTC recovery

The `daemon-rtc` deployment runs the actual ordinary daemon, RTC, Hub, Router and
a transient controller. The controller reads the real RTC through its namespace
entry and sends an authenticated private request to test Root. Root kills that
old RTC main once. Production Control admits the real daemon's Recover and
performs the entire retirement and replacement; the fixture never constructs or
launches a replacement.

The controller then obtains a fresh namespace token, verifies its `opened_by`
owner changed, and receives another real `rtc::client::now` reply. Generic
Control diagnostics record actual admission, construction and finalized
retirement; no device policy or test-name branch was added to System.

Root's final strict native runs passed on 4/8 harts in 2.43s/2.63s. Both used old
RTC Task/Team 15/15, actual daemon 16, and replacement Task/Team 18/18. Independent
verification requires the old actual trace reap before new spawn, Root-owned
construction, the daemon and controller alive across recovery, changed task and
team, fresh entry owner and actual time response, daemon retirement before RTC,
successful controller and actual boot-Root exits, normal halt and equal final
frame counts (87 -> 87 / 155 -> 155). All passed. Evidence:
`target/driver-composition/daemon/rtc-final-{4,8}/`, including retained raw qtest
logs and a separate verifier result. The checker has 22 passing positive and
negative controls.

The first fixture writer was interrupted with exit 130 after using prohibited
text-replacement scripts. Root archived its complete diff/new files, took over
with exact-context patches, corrected incomplete startup/ownership/compile
paths, and ran the builds and native checks. The read-only reviewer found no
confirmed blocker in the corrected fixture. The archived candidate itself was
not presented as accepted evidence.

## Remaining work

- ordinary non-DMA multi-member Team recovery and explicit-stop native races;
- native strict qtest 4/8-hart and product/Ruin/forced/accept regressions on the
  final source;
- full native acceptance of the broader refactor remains outside this phase.
