<p align="center">
  <img src="https://raw.githubusercontent.com/anreonyr/sqware/master/assets/sqware-logo.png" alt="sqware logo" width="520">
</p>

<p align="center">
  <strong>A structured world, made as software.</strong>
</p>

<p align="center">
  <a href="#design">Design</a> ·
  <a href="#core-structures">Core Structures</a> ·
  <a href="#protocol-and-service">Protocol & Service</a> ·
  <a href="#implementation-layers">Layers</a> ·
  <a href="#abi-surface">ABI</a> ·
  <a href="#boot-and-shutdown">Boot</a> ·
  <a href="#lisp-as-shell">Lisp Shell</a> ·
  <a href="#documentation">Docs</a> ·
  <a href="#status">Status</a>
</p>

# sqware

> A structured world, made as software.

**sqware** is an experimental microkernel for RISC-V, exploring a simple question:

> **What if a kernel provided structure rather than objects?**

Instead of defining a fixed kernel object for every kind of resource, sqware tries to provide a small set of orthogonal mechanisms from which higher-level system semantics can be built.

The goal is not simply to make a smaller operating system. It is to explore how heterogeneous hardware can be organized into **safe, composable multitasking interfaces**.

---

## Design

The central idea of sqware is:

> **The kernel provides structure; non-kernel software provides semantics.**

The kernel is responsible for the structures and boundaries that cannot safely be bypassed. It does not need to understand the complete meaning of every resource.

```text
                  Kernel
                    │
       ┌────────────┼────────────┐
       │            │            │
     Space         Task        Mail
       │                         │
      Map                        │
       │                         │
      Pie                        data
       │                         │
       └────────────┬────────────┘
                    │
             Non-Kernel Space
                    │
          ┌─────────┴─────────┐
          │                   │
       Protocol            Service
          │                   │
          └─────────┬─────────┘
                    │
                 System
                 semantics
```

The same mechanisms can therefore support different system-level abstractions without turning each abstraction into a new kernel object.

---

## Core Structures

### Space

A **Space** defines a world in which addresses and mappings exist.

It provides a structural boundary for memory and resource placement.

### Map

A **Map** describes how resources are placed into a Space.

It is concerned with mapping and placement rather than the higher-level meaning of the mapped resource.

### Task

A **Task** is a schedulable unit of execution.

Tasks provide the execution side of the system, while their surrounding structural relationships determine which Space and resources they operate within.

### Team

A **Team** organizes execution and ownership relationships between Tasks and Spaces.

It is not intended to be a universal "process object". It is a structural relationship from which process-like behavior can be constructed.

### Pie

A **Pie** carries authority to access a resource across Space boundaries.

Conceptually:

```text
Resource
Permission
Provenance
Lifetime
```

Its permission model currently uses four basic rights:

```text
R — Read
W — Write
V — Vest
B — Back
```

`R` and `W` say what may be done to the resource. `V` allows handing a copy of the authority to another Task. `B` restricts vesting to the original grantor, so a delegated authority cannot travel further than its holder intended.

Authority only ever moves as a **subset**: `Accord` grants a narrower copy, `Revoke` takes one back, `Narrow` reduces the holder's own copy in place. A Pie is addressed by a per-pie token that names it inside the holding Task's own table — a token is not self-proving, so holding somebody else's token is not a way around the check.

Pie provides the **authority mechanism**, but does not define what a resource means. The kernel enforces the authority; non-kernel protocols can give that authority higher-level semantics.

Authority does not have to be authority *over data*. The right to create a new Space is carried by a Pie whose resource has no data plane at all (see `Nole` below). Expressing it as a permission *bit* was rejected: a bit says "what may be done with this resource", which would let every resource carry the build right — and then "is this the right one?" can no longer be answered.

### Mail

**Mail** provides the data-transfer side of communication.

The model deliberately separates:

```text
Pie  → authority
Mail → data
```

The data plane comes in three types, grouped by **whether they have a data plane at all**:

```text
Hole — has a slot    single-message mailbox; the data passes through the kernel
Pole — has pages     page-granular secure memory; the data stays in mapped frames
Nole — has nothing   identity and lifetime only — no id, no mtu, no bytes
```

`Hole` and `Pole` are the structural endpoints of communication. `Nole` is the structural endpoint of **existence**: with nothing to send, receive or map, it cannot be mistaken for a resource and put to work as one — which is exactly what makes it the carrier for rights that are *not* about a resource. Its first consumer is the build right (`UnitCall::Build`).

This keeps communication and authority orthogonal: a protocol can decide what messages mean without requiring the kernel to understand the protocol itself.

---

## Protocol and Service

sqware separates **mechanism** from **system semantics**.

A **Protocol** defines a functional model: what operations exist, what they mean, and how components interact.

A **Service** implements that model.

```text
Protocol
   │
   │ defines
   ▼
Semantics
   ▲
   │ implements
   │
Service
```

For example, a directory does not need to be a special kernel object. A non-kernel Directory Service can implement a Directory Protocol using the primitives provided by sqware.

Likewise, a protocol may define its own capability semantics. The kernel does not need to know names such as `Lookup`, `Publish`, or `Remove`; it only needs to enforce the underlying authority represented by Pie.

This makes sqware **capability-oriented without requiring a capability-object hierarchy in the kernel**.

### Implemented today

Two protocols exist, both as ordinary non-kernel code:

| Protocol | Where | Operations |
|---|---|---|
| **Directory** | `crates/protocol/src/dispatch/` | `Register` `Unregister` `Replace` `Resolve` `Enumerate` `Connect`, plus one parent-side action `Refer` |
| **Console** | `crates/protocol/src/console/` | `Open` `Write` `ReadLine` `Close` |
| **Doom** | `crates/protocol/src/doom/` | `Kill`, plus the owner-only `Quit` |
| **IRQ** | `crates/protocol/src/irq/` | `Register` (a client claims the line of a device *by name*), plus the parent-side `Refer` (root writes the owner) |

The kernel holds none of their code and has no entry call for them: the directory is reached through a request hole, and every operation is a message on it. Rendering, key decoding and line editing live on the console **service** side; a client only says "I wrote this" and "give me a line".

The directory's central structure is a **reservation table**, not a registry. Rows are created only by the parent domain (`Refer`), and `Register` can only *fill* an existing row — so "who may use which name" is a property of the table's shape rather than a runtime decision.

The interrupt driver's table has the same shape, with one more column: `name → (line, owner, instance)`. The line number is **a function of the name** (`interrupts` × `interrupt-parent` read out of the device tree by the driver), so a client cannot state — or steal — a line: the register message has no line field at all. Its owner is written only by root, the device name is relayed by root, and a delivery that fails (`Denied`/`Dead`) makes the driver close the line and drop the instance while keeping the row.

Services are Teams:

| Program | Space | Role |
|---|---|---|
| `prog-root` | S-mode | the only domain built at boot; builds every other program |
| `prog-console` | S-mode | the only task that reads the UART |
| `prog-dir` | S-mode | the directory service |
| `prog-echo` | S-mode | a sample service |
| `prog-shell` | U-mode | the interactive shell — a client of the directory |

The kernel answers exactly one question about killing: *may this domain kill that one* — by
**lineage**, transitively (`RoomCall::Doom`). "Should it" is policy and lives in a service:
root is the ancestor of every domain, so it is the one that qualifies, and it exposes a
`doom` service (`kill <name>`) — the shape of Unix `kill`, with the mechanism in the kernel
and the check in a service rather than in a system call.

---

## Implementation Layers

```text
kernel     the S-mode microkernel: structure, isolation, enforcement
   ↑
env        the ABI: envcall declarations, payload codec, permission bits
   ↑
runtime    mechanisms: channels, handshake, heap, locks, TLS, units
   ↑
protocol   semantics: the directory and console protocols
   ↑
programs   assembled images: root, console, dir, echo, shell
```

| Path | Role |
|---|---|
| `kernel/` | the S-mode microkernel (`no_std`) |
| `crates/env/` | ABI surface: envcall classes, payload codec, `Permission` (single source of truth) |
| `crates/envmacros/` | the `#[call(class = …)]` macro that turns a declaration into a codec |
| `crates/runtime/` | mechanism layer: `core/{channel,handshake,heap,lock,tls,unit}` + `env/*` wrappers |
| `crates/protocol/` | protocol semantics: `dispatch/` (directory) + `console/` |
| `crates/sbi/` | SBI access for platform services |
| `programs/` | the images packed into the initrd |
| `scripts/` | `boot.nu` (the single source of QEMU launch), `runner.nu` (cargo integration), `examine.nu` (acceptance gate), `quick.sh` (dev round-trip) |

The kernel is organized around the same words it exposes:

```text
kernel/src/
  boot.rs machine.rs layout.rs initrd.rs health/
  memory/   allocator/{bitmap,block,bump,hybrid,spare,portal} + manager/{table,asid,fault}
  work/
    room/   conductor (halt criterion) + scheduler/{hart,ident,table,fetch} + messenger/
    unit/   space/ map/ gate/{pie,narrow,accord,revoke,release} task team loader parser life
    mail/   hole pole nole
  runtime/  switcher/{trap,envcall} chrono/ diagnose/
  lock/     spin rw reentrant once lazy + lockdep
```

---

## ABI Surface

An envcall slot is `(class << 32) | index`, where `index` is declaration order.

| class | name | operations |
|---|---|---|
| 0 | Room | `Starve` `Park` `Reap` `Wait` `Wake` |
| 1 | Unit | `Spawn` `SelfId` `Sire` `HeirCount` `Heir` `Build` `Hatch` `Join` |
| 2 | Memory | `Allocate` `Deallocate` `Mmap` `Munmap` `Mprotect` |
| 3 | IO | `Get` `Put` |
| 4 | Chrono | `Ticks` `Clock` |
| 5 | Mail | `Push` `Pull` `Wait` |
| 6 | Control | `Backtrace` |
| 7 | Pie | `UnsealHole` `UnsealPole` `UnsealNole` `Open` `Shut` `Seal` `Accord` `Narrow` `Revoke` `Collect` `Reserve` `Release` |

Three properties are worth naming:

- **The axes are separated.** Mail is the **data** axis — what crosses a hole. Pie is the **authority** axis — the lifetime and flow of permissions. Neither carries the other's payload; they used to share one class, and pulling them apart is what made the split visible.
- **Termination has a single exit.** `RoomCall::Reap { reason }` ends a task, and the reason code is *data, not policy*: the kernel records it and does not interpret it. There is deliberately no separate "panic call" — a domain that cannot continue is still just a domain that stops, and the kernel does not need to know the word "panic".
- **An unknown call is refused, not fatal.** Undeclared classes, reserved indices and out-of-range slots all decode to `BadSlot` and are rejected, so an illegal call number never takes down the machine.

Authority is also checked where it is used, not where it is declared: `Build` asks for a token and two gates answer — the **capability** ("who may"), and the **S-mode** fallback ("may the blood line reach past the sandbox at all").

---

## Boot and Shutdown

```text
boot
 │  exactly one S-mode domain is built: root
 ▼
root ──Build──► dir / echo / console   (services)
 │  ──Build──► shell                   (U-mode client)
 │  handshake: Quay (child→parent) · Pier (parent→child)
 │             Refer / Reserve (parent→dir) · Referred (dir→parent)
 ▼
shell exits ⇒ root exits
 ▼
doom cascade stops and reaps the whole blood line
 ▼
every task reaped ⇒ conductor::done ⇒ srst
```

Two structural decisions are visible here:

- **No external timeout.** The machine resets because the task count balances to zero, not because a host-side timer fired. `root` exiting is enough to bring everything down with it.
- **Boot is the only special case.** After `root` exists, services, clients and authority are all produced by ordinary `Build` / `Spawn` / `Hatch` / `Accord` calls from non-kernel code. The initrd is a temporary mechanism: the kernel maps it read-only into `root` and does not know its format — the manifest is interpreted by the root program, which then hands each image to `Build` unchanged.

---

## Structure over Objects

sqware deliberately avoids building its architecture around a large collection of predefined kernel objects.

Instead of:

```text
Device
File
Socket
Channel
MemoryObject
DeviceCapability
FileCapability
...
```

the kernel provides a smaller structural vocabulary:

```text
Space
Map
Task
Team
Pie
Mail
```

Higher-level software can then compose these mechanisms into the abstractions it needs.

This does not mean that higher-level objects disappear. They move to the layer where their semantics belong: **Protocols and Services outside the kernel**.

The result is a distinction between:

```text
Kernel mechanism
        │
        ▼
Protocol semantics
        │
        ▼
Service implementation
```

System complexity can therefore grow through composition rather than requiring the kernel's object model to grow with every new feature.

---

## Hardware as Multitasking Interfaces

One of the main questions behind sqware is how hardware can be integrated into a multitasking system without requiring the kernel to define the complete semantics of every device.

Conceptually:

```text
Hardware
    │
    ▼
Structural representation
    │
    ├── Space
    ├── Map
    ├── Pie
    └── Mail
    │
    ▼
Task
    │
    ▼
Protocol
    │
    ▼
Service
    │
    ▼
System semantics
```

The kernel therefore acts as a structural substrate between hardware and higher-level software.

A driver, service, or other non-kernel component can decide how a particular resource should behave, while the kernel remains responsible for the fundamental boundaries between spaces, execution, mappings, authority, and communication.

The console service is the first instance of this: the kernel's whole device surface is a byte-level IO call, and the entire terminal — rendering, escape decoding, line editing, per-client sessions — is a service. Device ownership belongs to a driver that does not exist yet; the one call it will replace is already isolated and has a single caller.

---

## Lisp as Shell

The intended shell direction for sqware is **Lisp**.

This is not simply a choice of command syntax. Lisp is a natural environment for the same compositional model used by the system itself.

Instead of treating the shell as a collection of commands, a Lisp shell can treat system construction as composition:

```lisp
(service
  (name "directory")
  (requires
    (capability "storage"))
  (provides
    (protocol "directory")))
```

The Lisp layer does not need to know how a Task is scheduled, how a Space is isolated, or how a Pie is enforced. Those are kernel mechanisms. It describes the system that should be constructed from them.

In this sense:

> **The kernel provides the building blocks; Lisp composes them.**

The shell becomes another expression of the same principle: system complexity comes from composition rather than from continually adding special cases to the kernel.

### Where the shell stands today

The shell that ships now is a Rust REPL in a U-mode domain (`programs/src/bin/user/shell.rs`), and it is already a *client* rather than a privileged component: it discovers services through the directory protocol, reads and writes through the console service, and holds no authority it was not granted. Lisp is the intended composition layer above that shell — a direction, not yet an implementation.

---

## Design Principles

### 1. Structure over Objects

Prefer general structural mechanisms over a growing hierarchy of special-purpose kernel objects.

### 2. Mechanism over Policy

The kernel should enforce fundamental constraints without deciding every higher-level system policy.

### 3. Orthogonality

Core mechanisms should have independent responsibilities and compose rather than duplicate each other.

### 4. Authority is Explicit

Access to a resource should come from explicit authority rather than ambient identity.

### 5. Data and Authority are Separate

`Mail` carries data.

`Pie` carries authority.

Neither should need to become the other — and a Pie whose resource has no data plane (`Nole`) is the same principle in its purest form: authority that is about *existing* rather than about *access*.

### 6. Semantics Belong Above the Kernel

The kernel provides mechanisms from which resource-specific semantics can be constructed.

### 7. Composition over Specialization

New system behavior should, where possible, be constructed by composing existing structures and protocols instead of introducing another kernel primitive.

---

## Architecture

At the current stage, the conceptual organization is:

```text
                       Team
                    /        \
                Space         Task
                  │             │
                 Map           Pie ──── Nole (no data plane)
                                │
                                │ authority
                                ▼
                              Mail
                                │
                                │ data
                                ▼
                       Non-Kernel Space
                          /          \
                     Protocol      Service
                          \          /
                           \        /
                            System
```

The important boundary is not a hierarchy of kernel objects, but the division of responsibility:

```text
Kernel
  → structure, isolation, enforcement

Non-Kernel
  → semantics, protocols, services, composition
```

These are **mechanisms and relationships**, not a conventional kernel-object hierarchy.

---

## Why "sqware"?

`sqware` comes from:

```text
square + sphere + ware
```

### square

Structure, geometry, discreteness, and composition.

### sphere

A world, a space, and the boundary within which things exist and interact.

### ware

A constructed system — software, hardware, and the things built from them.

Together:

> **sqware — a structured world, made as software.**

---

## Documentation

The mechanism-by-mechanism design docs live in [`docs/`](docs/README.md), and they follow the
same order as this README: structure first, then semantics, then the base everything stands on.

| Structure | Semantics | Base |
|---|---|---|
| [`docs/space.md`](docs/space.md) — Space / Map / Window / Seg | [`docs/dispatch.md`](docs/dispatch.md) — the directory protocol | [`docs/switcher.md`](docs/switcher.md) — traps, switching, the envcall entry |
| [`docs/memory.md`](docs/memory.md) — frames, page tables, audit | [`docs/console.md`](docs/console.md) — the console protocol & service | [`docs/abi.md`](docs/abi.md) — the ABI surface and the layers |
| [`docs/task.md`](docs/task.md) — Task / Team / lineage / scheduler | [`docs/root.md`](docs/root.md) — the root domain | [`docs/lock.md`](docs/lock.md) — locks and lockdep |
| [`docs/pie.md`](docs/pie.md) — authority | [`docs/driver.md`](docs/driver.md) — devices as owned memory, the interrupt gate | [`docs/diagnose.md`](docs/diagnose.md) — diagnosis and crash scenes |
| [`docs/mail.md`](docs/mail.md) — Hole / Pole / Nole | | |

Every doc has the same shape: where the mechanism sits and what it refuses to do, what it is made
of, its invariants (with what breaks if one is violated), one complete sequence, the decisions
behind it, its known edges, and — last but not least — **how it is known to be correct**: the
self-test, the acceptance-gate assertion, or the shutdown accounting that would catch it.

## Status

sqware is an **experimental and actively evolving** microkernel. It targets **RISC-V** (S-mode, `riscv64gc-unknown-none-elf`) and is implemented in **Rust** (`no_std`).

What runs end to end today, on QEMU `riscv-virt`:

```text
boot → root domain → dir / echo / console services → U-mode shell
     → spawn · wait · mail · clock · memory · directory calls
     → authority cascade: revoke · narrow · release · task death
     → illegal slots and stray ids rejected, kernel alive
     → exit → doom cascade → all tasks reaped → srst (natural reset)
```

Verification is a gate rather than a smoke test (`scripts/examine.nu`): it builds each flavor, drives the guest command by command — waiting for each expected output instead of racing a wall clock — and requires natural shutdown (never a host timeout), no kernel panic, and a full set of markers.

```nu
nu scripts/examine.nu                                          # default, 3 rounds
EXAMINE_FEATURES=audit nu scripts/examine.nu                   # + one accounting round
EXAMINE_FEATURES=audit EXAMINE_HARDEN=1 nu scripts/examine.nu  # full gate
```

Three flavors cover different halves of the guardrail:

| Flavor | Artifact | What it adds |
|---|---|---|
| **default** | release | the behavioural contract, three times over |
| **audit** | release + `--features audit` | per-object accounting at shutdown: frames, pages, wait sites, live tasks |
| **harden** | `--profile harden` = release + `debug_assertions` (+ `audit`) | container/state invariants and the full lockdep checker, compiled into the artifact under test |

The harden artifact is inspected before it is run: the gate greps it for all four guardrail strings — the assertion, the ledger, the frame-side chain invariant, and the lockdep report. A flavor that quietly lost half its guardrail would otherwise just be another round of a different flavor.

Build and run:

```sh
cargo build --release   # kernel + programs + initrd
cargo run --release      # boots in QEMU (via scripts/runner.nu)
scripts/quick.sh dir     # one cold boot and a round-trip, no verdict
```

The architecture is still being developed. APIs, abstractions, and terminology may change as the design is tested against real hardware and increasingly complex multitasking scenarios.

Not there yet, by design or by order of work: a **device-class framework** (what landed is the *basis* — a device is owned, mappable memory, and one interrupt line per hop; see [`docs/driver.md`](docs/driver.md)), a file service (the initrd is explicitly temporary), and the Lisp composition layer.

The project is intentionally experimental:

> **The implementation is part of the exploration.**

---

## What sqware is trying to discover

The project is built around a few open questions:

- How little structure is required to build a useful multitasking system?
- Can heterogeneous hardware be expressed without a large kernel object hierarchy?
- Can authority be represented generically rather than through resource-specific capability types?
- How much operating-system semantics can be moved above the kernel?
- Can Protocols and Services provide rich system behavior without becoming part of the kernel object model?
- Can a structural kernel remain understandable as the system grows?
- Can a compositional shell make system construction itself programmable?
- Where is the boundary between hardware mechanism and operating-system policy?

These questions are more important to the project than any particular API.

---

## Philosophy

sqware does not try to answer:

> "What should an operating system look like?"

It asks a smaller question:

> **"What structure is necessary for hardware to become a multitasking system?"**

Everything else should, as far as possible, grow from that structure.

The implementation follows the same idea at multiple levels:

> **Structure in the kernel. Semantics above it. Composition everywhere.**

---

## License

See the repository for the current license and project details.
