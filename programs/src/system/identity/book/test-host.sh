#!/bin/sh
# Run from any directory. No arguments.
set -eu
if [ "$#" -ne 0 ]; then
    echo "usage: $0 (no arguments)" >&2
    exit 2
fi
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../../../../.." && pwd)
host=${HOST_TARGET:-x86_64-unknown-linux-gnu}
cd "$root"
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
python3 - "$here" "$temporary" "$root" <<'PY'
import pathlib, sys
here, temporary, root = map(pathlib.Path, sys.argv[1:])
env_url = "https://github.com/shuyra-arche/env.git"
env_rev = "d3000253cd5d13d466e91fafd4dbb27a5ec5aaae"
system_url = "https://github.com/shuyra-arche/system.git"
system_rev = "9e5d63ca536a176583cf1b0054b4ecf7ddd88257"
source = (root / "programs/tests/identity/host.rs").read_text()
source = source.replace('#[path = "../../src/system/identity/service/answer.rs"]', f'#[path = "{here / "../service/answer.rs"}"]')
source = source.replace('#[path = "../../src/system/identity/book/mod.rs"]', f'#[path = "{here / "mod.rs"}"]')
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
env = {{ git = "{env_url}", rev = "{env_rev}" }}
system-api = {{ git = "{system_url}", rev = "{system_rev}" }}
''')
PY
cargo test --manifest-path "$temporary/Cargo.toml" --target "$host" \
    --target-dir "$temporary/target" --offline
