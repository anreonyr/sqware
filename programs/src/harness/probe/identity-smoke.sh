#!/bin/sh
# Actual Identity IPC and a delayed read after client timeout, without device consumers.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../../../.." && pwd)
cd "$root"
cargo image accept debug
cargo build -p kernel
mkdir -p "$root/target"
temporary=$(mktemp -d "$root/target/identity-smoke.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
python3 - "$root" "$here" "$temporary" <<'PY'
import json, pathlib, sys
root, here, temporary = map(pathlib.Path, sys.argv[1:])
(temporary / "Cargo.toml").write_text(f'''
[package]
name = "identity-smoke-image"
version = "0.0.0"
edition = "2024"
[workspace]
[[bin]]
name = "identity-smoke-image"
path = {json.dumps(str(here / "identity-smoke.rs"))}
[dependencies]
env = {{ path = {json.dumps(str(root / "crates/env"))} }}
''')
PY
image="$root/target/riscv64gc-unknown-none-elf/debug"
cargo run --manifest-path "$temporary/Cargo.toml" --target x86_64-unknown-linux-gnu \
    --target-dir "$temporary/build" --offline -- "$image/initrd.img" "$temporary/initrd.img"
cp "$image/sqware" "$temporary/sqware"
if QEMU_TIMEOUT=30 QEMU_SETTLE=0 QEMU_SEMI=1 \
    nu scripts/boot.nu "$temporary/sqware" </dev/null >"$temporary/qemu.log" 2>&1
then
    status=0
else
    status=$?
fi
cat "$temporary/qemu.log"
test "$status" -eq 0
if grep -Eq 'panic|system: assemble|system: doom|forcing shutdown' "$temporary/qemu.log"
then
    echo "identity smoke: failure in QEMU log" >&2
    exit 1
fi
for note in \
    "subject: lineage and narrowing held" \
    "member: manager, inactive selection and pages held" \
    "probe-denied: installer and action isolation held" \
    "probe-coalition: identity=17, all action entries home" \
    "probe-control: ask open, three faces denied" \
    "probe-bound: doors held against junk" \
    "system: done"
do
    grep -F "reason=0x0 note: $note" "$temporary/qemu.log" >/dev/null
done
grep -F "identity-timeout: queued request decoded after client timeout" "$temporary/qemu.log" >/dev/null
echo "identity smoke: six real IPC probes and clean system shutdown passed"
