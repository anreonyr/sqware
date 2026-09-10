<p align="center">
  <img src="assets/sqware-logo.svg" alt="sqware logo" width="520">
</p>

<p align="center">
  <strong>A structured world, made as software.</strong>
</p>

<p align="center">
  <a href="#design">Design</a> ·
  <a href="#core-structures">Core Structures</a> ·
  <a href="#protocol-and-service">Protocol & Service</a> ·
  <a href="#lisp-as-shell">Lisp Shell</a>
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
      Map                         │
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

Pie provides the **authority mechanism**, but does not define what a resource means. The kernel enforces the authority; non-kernel protocols can give that authority higher-level semantics.

### Mail

**Mail** provides the data-transfer side of communication.

The model deliberately separates:

```text
Pie  → authority
Mail → data
```

`Hole` and `Pole` form the structural endpoints of this communication mechanism.

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

The kernel provides mechanisms from which resource-specific semantics can be constructed.

### 7. Composition over Specialization

New system behavior should, where possible, be constructed by composing existing structures and protocols instead of introducing another kernel primitive.

---

## Architecture

At the current stage, the conceptual organization is:

```text
                    Team
                 /         \
             Space         Task
               │             │
              Map           Pie
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
