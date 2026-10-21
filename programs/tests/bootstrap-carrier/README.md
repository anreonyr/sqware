# Bootstrap carrier

Run from the repository root:

```sh
cargo test --manifest-path programs/tests/bootstrap-carrier/Cargo.toml --target x86_64-unknown-linux-gnu --offline
```

The suite compiles the production Assembly carrier and client decoder. The ABI and
wire frame are real; effects are host fixtures. The schedule test executes the real
Pending cursor behavior, including each failed carrier boundary and later-step gating.
It checks the same root across retries and failed release, a fixed timeout before new
publication, independent children, exact permissions, and untrusted frames.

Control's whole-Team completion and failed-prepare transitions are covered separately
by control-instance. Kernel Creator authority and actual boot/reclamation require the
native tests and product/soak scenes recorded in docs/driver-composition.md.
