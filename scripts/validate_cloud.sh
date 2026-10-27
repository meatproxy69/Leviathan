#!/usr/bin/env bash
# Offline reproducible checks. No system installation, device opening, or global settings changes.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
export CARGO_HOME="${CARGO_HOME:-$ROOT/.cargo-home}"
if [[ -x "$ROOT/.cargo-home/bin/cargo" && -d "$ROOT/.toolchain/rustup" ]]; then
  export RUSTUP_HOME="${RUSTUP_HOME:-$ROOT/.toolchain/rustup}"
  export PATH="$ROOT/.cargo-home/bin:$PATH"
fi

capture=false
if [[ $# == 0 ]]; then
  mkdir -p evidence
  destination="$(mktemp -d "$ROOT/evidence/cloud-validation.XXXXXX")"
elif [[ $# == 2 && "$1" == --capture ]]; then
  capture=true
  destination="$2"
  # A NEW directory is required; do not replace earlier evidence.
  mkdir "$destination"
  destination="$(cd "$destination" && pwd)"
else
  echo "usage: scripts/validate_cloud.sh [--capture NEW-EVIDENCE-DIRECTORY]" >&2
  exit 2
fi
echo "Writing validation evidence to $destination"
exec > >(tee "$destination/validation.log") 2>&1

hash_sources() {
  python3 - <<'PY'
from pathlib import Path
import hashlib
files = [Path('Cargo.toml'), Path('Cargo.lock')]
for folder in ('src', 'tests', 'examples', 'scripts'):
    files.extend(p for p in Path(folder).rglob('*') if p.is_file() and '__pycache__' not in p.parts)
for path in sorted(files):
    print(hashlib.sha256(path.read_bytes()).hexdigest(), path.as_posix())
PY
}
hash_sources > "$destination/sources.sha256"
cargo --version
rustc --version
cargo tree --locked --offline --no-default-features --features desktop --edges normal \
  --prefix none --format '{p}' > "$destination/desktop-dependencies.txt"
cargo tree --locked --offline --no-default-features --features desktop-audio --edges normal \
  --prefix none --format '{p}' > "$destination/desktop-audio-dependencies.txt"
python3 - "$destination" <<'PY'
from pathlib import Path
import sys
root = Path(sys.argv[1])
names = lambda filename: {line.split()[0] for line in (root / filename).read_text().splitlines() if line.strip()}
headless = names('desktop-dependencies.txt')
host = names('desktop-audio-dependencies.txt')
forbidden = {'cpal', 'alsa', 'alsa-sys', 'jack', 'jack-sys'}
assert not (headless & forbidden), f'audio backend leaked into desktop feature: {headless & forbidden}'
assert 'eframe' in headless and 'phonic' in headless
assert 'cpal' in host, 'desktop-audio must retain the physical audio backend'
print('Feature graph verified: desktop has GUI/offline audio only; desktop-audio retains CPAL.')
PY

cargo fmt --all -- --check
cargo test --locked --offline --no-default-features --all-targets
cargo test --locked --offline --no-default-features --features desktop --all-targets
cargo clippy --locked --offline --no-default-features --features ui-capture --all-targets -- -D warnings
cargo build --locked --offline --no-default-features --features desktop --bin leviathan

if $capture; then
  cargo build --locked --offline --no-default-features --features ui-capture --example ui_capture
  for size in 1180x780 900x620; do
    width="${size%x*}"
    height="${size#*x}"
    for scene in shell edit follow; do
      name="$destination/$scene-$size"
      MESA_SHADER_CACHE_DISABLE=true "$ROOT/target/debug/examples/ui_capture" "$name.png" "$width" "$height" "$scene" "$name.json"
    done
  done
fi
hash_sources > "$destination/sources-after.sha256"
if ! cmp -s "$destination/sources.sha256" "$destination/sources-after.sha256"; then
  echo 'Source changed during validation; rerun after edits settle. These are not final-revision results.' >&2
  exit 1
fi
echo 'PASS: cloud checks on unchanged source. Physical audio, OS-window/input, and full-app Follow remain unverified.'
