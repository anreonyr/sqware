# Concurrent weak-copy bookkeeping

Run `cargo test --manifest-path kernel/tests/weak-copies/Cargo.toml --target x86_64-unknown-linux-gnu`.

The fixture supplies only the hart identity. Tests include the actual production
tracker and exercise eight concurrent harts, genuine held-copy failures and
stored-to-temporary slot reuse. The concurrent test reproduced false Muster
assertions against the former split occupancy/hart/origin implementation on the
host; the single-record implementation passes the same test. Native four/eight
hart daemon scenes independently retain the normal halt and frame equality gates.
