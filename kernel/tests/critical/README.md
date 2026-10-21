# Execution-domain stop evidence

The `critical-scene` feature adds one test that must remain halted. Its normal
`cargo qtest` result is a timeout and exit 101. A timeout alone is not acceptance.
The separate verifier requires the guard event, every peer's quiescence receipt,
their ordering, and absence of the target's actual reference-cleanup event.

Run the commands separately, with no concurrent QEMU builds:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true QEMU_SMP=4 TRACE_OUT="$PWD/target/driver-composition/guard/critical-4-online" nu scripts/qtest.nu --package kernel -- --features semihosting,critical-scene --timeout 8 -- critical_domain_exit_stops
python3 kernel/tests/critical/verify.py target/driver-composition/guard/critical-4-online/qtest.log 4
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true QEMU_SMP=8 TRACE_OUT="$PWD/target/driver-composition/guard/critical-8-online" nu scripts/qtest.nu --package kernel -- --features semihosting,critical-scene --timeout 8 -- critical_domain_exit_stops
python3 kernel/tests/critical/verify.py target/driver-composition/guard/critical-8-online/qtest.log 8
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s kernel/tests/critical
```

The dedicated case starts the real secondary harts. Quiescence is acknowledged
after a peer vacates its address-space lease and disables interrupts, before its
non-returning HSM stop or fallback loop. Only then does the initiating hart park.
The alarm's atomic owner chooses the coordinator; the alarm itself is not an
acknowledgement that peers or devices have stopped.

This kernel test slays a held protected task. It does not prove that deployed
drivers are protected or that device DMA was drained. Control deployment wiring
and unexpected driver-death scenes remain separate acceptance requirements.
