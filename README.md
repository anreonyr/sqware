# sqware

> A structured world, made as software.

**sqware** is an experimental microkernel for RISC-V, exploring a simple question:

> **What if a kernel provided structure rather than objects?**

Instead of defining a fixed kernel object for every kind of resource, sqware tries to provide a small set of orthogonal mechanisms from which higher-level system semantics can be built.

The goal is not simply to make a smaller operating system. It is to explore how heterogeneous hardware can be organized into **safe, composable multitasking interfaces**.

---

## Design

The central idea of sqware is:

> **The kernel provides structure; higher-level software provides semantics.**

A hardware resource does not necessarily need to become a special kernel object. Instead, it can be organized through a small set of structures:

```text
                    Hardware
                       │
                       ▼
                 ┌───────────┐
                 │   Space   │
                 └─────┬─────┘
                       │
                      Map
                       │
              ┌────────┴────────┐
              │                 │
             Task             Pie
              │                 │
              └────────┬────────┘
                       │
                     Mail
                       │
                Multitasking
```

The same mechanisms can then be composed into different system-level abstractions.

---

## Core Structures

### Space

A **Space** defines a world in which addresses and mappings exist.

It provides the structural boundary for memory and resource placement.

### Map

A **Map** describes how resources are placed into a Space.

It is concerned with the relationship between an address space and the resources mapped into it, rather than with the higher-level meaning of those resources.

### Task

A **Task** is a schedulable unit of execution.

Tasks belonging to the same Team share its Space while maintaining their own execution state.

```text
Team
 ├── Space
 ├── Task
 ├── Task
 └── Task
```

### Team

A **Team** organizes Tasks around a common Space.

It provides the structural relationship between an execution group and its address-space world.

A Team is not intended to be a universal "process object"; it is primarily a structural relationship between execution and space.

### Pie

A **Pie** carries authority to access a resource.

A Pie combines:

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

The important distinction is that Pie does not define what a resource *means*. It provides the mechanism through which authority over a resource can be held and transferred.

### Mail

**Mail** provides the data-transfer side of communication.

The current model separates:

```text
Pie  → authority
Mail → data
```

`Hole` and `Pole` form the structural endpoints of this communication mechanism.

This separation allows communication and authorization to remain independent.

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

This is a design choice, not a claim that every operating system should work this way.

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
Higher-level service
    │
    ▼
System semantics
```

The kernel therefore acts as a structural substrate between hardware and higher-level software.

A device driver or system service can decide how a particular resource should behave, while the kernel remains responsible for the fundamental boundaries between spaces, execution, mappings, authority, and communication.

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

Neither should need to become the other.

### 6. Semantics Belong Above the Kernel

The kernel provides the mechanisms from which resource-specific semantics can be constructed.

---

## Architecture

At the current stage, the conceptual organization is:

```text
sqware
│
├── Team
│   ├── Space
│   │   └── Map
│   │
│   └── Task
│       └── Pie[]
│
└── Mail
    ├── Hole
    └── Pole
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

## Status

sqware is an **experimental and actively evolving** microkernel.

It currently targets **RISC-V** and is implemented primarily in **Rust**.

The architecture is still being developed. APIs, abstractions, and terminology may change as the design is tested against real hardware and increasingly complex multitasking scenarios.

The project is intentionally experimental:

> **The implementation is part of the exploration.**

---

## What sqware is trying to discover

The project is built around a few open questions:

- How little structure is required to build a useful multitasking system?
- Can heterogeneous hardware be expressed without a large kernel object hierarchy?
- Can authority be represented generically rather than through resource-specific capability types?
- How much operating-system semantics can be moved above the kernel?
- Can a structural kernel remain understandable as the system grows?
- Where is the boundary between hardware mechanism and operating-system policy?

These questions are more important to the project than any particular API.

---

## Philosophy

sqware does not try to answer:

> "What should an operating system look like?"

It asks a smaller question:

> **"What structure is necessary for hardware to become a multitasking system?"**

Everything else should, as far as possible, grow from that structure.

---

## License

See the repository for the current license and project details.
