# Bounded PCI discovery primitives

Run `cargo test --manifest-path programs/tests/pci-scan/Cargo.toml --target x86_64-unknown-linux-gnu --offline`.

Tests compile the production scanner, snapshot parser and raw read-only window.
Allocated byte buffers exercise actual volatile reads and mapped-range edges;
the sparse fixture covers high requester IDs, bounded work, capacity continuation
and read failures. Both use the same production `Reader` contract.

The raw window constructor is unsafe: its caller keeps the aligned mapping alive
and readable. The window cannot be sent or shared across execution contexts.
The function limit comes from the actual mapped size, capped by the
16-bit requester space. Invalid cursors and failed reads return failure rather
than successful EOF or a missing function. Snapshot reads cover exactly 256 bytes.

This is an internal primitive checkpoint. PCI RPC discovery and protocol-driver
selection still require integration and native acceptance.
