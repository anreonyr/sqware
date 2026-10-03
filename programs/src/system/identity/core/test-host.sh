#!/bin/sh
# Run from any directory. Optional argument: another checkout containing protocol.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../../../../.." && pwd)
protocol_root=${1:-$root}
host=${HOST_TARGET:-x86_64-unknown-linux-gnu}
cd "$root"
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
python3 - "$here" "$protocol_root" "$temporary" "$root" <<'PY'
import pathlib, sys
here, protocol, temporary, root = map(pathlib.Path, sys.argv[1:])
source = (here / "host.rs").read_text()
source = source.replace("../../../../../crates/protocol", str(protocol / "crates/protocol"))
source = source.replace('#[path = "mod.rs"]', f'#[path = "{here / "mod.rs"}"]')
source = source.replace('#[path = "../serve/answer.rs"]', f'#[path = "{here / "../serve/answer.rs"}"]')
(temporary / "host.rs").write_text(source)
(temporary / "Cargo.toml").write_text(f'''
[package]
name = "identity-semantic-tests"
version = "0.0.0"
edition = "2024"
[workspace]
[lib]
path = "host.rs"
doctest = false
[dependencies]
env = {{ path = "{root / "crates/env"}" }}
''')
PY
cargo test --manifest-path "$temporary/Cargo.toml" --target "$host" \
    --target-dir "$temporary/target" --offline
