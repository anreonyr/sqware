#!/bin/sh
# Run from any directory. Optional argument: another checkout containing the API.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../../../../.." && pwd)
api_root=${1:-$root}
host=${HOST_TARGET:-x86_64-unknown-linux-gnu}
cd "$root"
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
python3 - "$here" "$api_root" "$temporary" "$root" <<'PY'
import pathlib, sys
here, api, temporary, root = map(pathlib.Path, sys.argv[1:])
source = (root / "programs/tests/identity/host.rs").read_text()
source = source.replace('#[path = "mod.rs"]', f'#[path = "{here / "mod.rs"}"]')
source = source.replace('#[path = "../serve/answer.rs"]', f'#[path = "{here / "../service/answer.rs"}"]')
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
system-api = {{ path = "{api / "programs/src/system/api"}" }}
''')
PY
cargo test --manifest-path "$temporary/Cargo.toml" --target "$host" \
    --target-dir "$temporary/target" --offline
