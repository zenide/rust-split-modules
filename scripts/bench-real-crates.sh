#!/usr/bin/env bash
#
# Reproduce the "Validation on real crates" table in the README.
#
# For each crate it: clones a shallow copy, runs the crate's own test suite as a
# baseline, splits every eligible file recursively with cargo-split-modules, then runs
# the test suite again. Identical test counts before/after prove behaviour is preserved.
# Files that can't be split safely are auto-rolled-back (reported in the "rolled back"
# column) and never break the build.
#
# Requirements: a Rust toolchain (cargo), git, and network access.
# Usage:        scripts/bench-real-crates.sh
# Env:          WORK=/path  override the scratch dir   OUT=/path/results.md  override output
#
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"   # repo root
TOOL="$HERE/target/release/cargo-split-modules"
WORK="${WORK:-$(mktemp -d)}"
OUT="${OUT:-$WORK/results.md}"
mkdir -p "$WORK"; : > "$OUT"

if [ ! -x "$TOOL" ]; then
  echo "building cargo-split-modules (release)…" >&2
  (cd "$HERE" && cargo build --release --quiet) || { echo "build failed" >&2; exit 1; }
fi

# crate | git url
CRATES=(
  "semver|https://github.com/dtolnay/semver"
  "bytes|https://github.com/tokio-rs/bytes"
  "anyhow|https://github.com/dtolnay/anyhow"
  "httparse|https://github.com/seanmonstar/httparse"
  "base64|https://github.com/marshallpierce/rust-base64"
  "memchr|https://github.com/BurntSushi/memchr"
  "bitflags|https://github.com/bitflags/bitflags"
  "heck|https://github.com/withoutboats/heck"
)

loc()    { find "$1" -name '*.rs' -print0 2>/dev/null | xargs -0 cat 2>/dev/null | wc -l | tr -d ' '; }
nfiles() { find "$1" -name '*.rs' 2>/dev/null | wc -l | tr -d ' '; }

# Echoes "STATUS COUNT", e.g. "ok 123", "FAIL -", "builderr -".
runtests() {
  local dir="$1" log
  log=$(cd "$dir" && cargo test 2>&1)
  if echo "$log" | grep -q "test result: FAILED"; then echo "FAIL -"; return; fi
  if echo "$log" | grep -qE "^error(\[|:)"; then echo "builderr -"; return; fi
  if ! echo "$log" | grep -q "test result: ok"; then echo "notests 0"; return; fi
  local n
  n=$(echo "$log" | grep -oE "test result: ok\. [0-9]+ passed" | grep -oE "[0-9]+" | awk '{s+=$1} END{print s}')
  echo "ok ${n:-0}"
}

printf '| crate | files before | files after | total LOC | avg LOC/file before | avg LOC/file after | files split | rolled back | tests before | tests after |\n' >> "$OUT"
printf '|---|---|---|---|---|---|---|---|---|---|\n' >> "$OUT"

for entry in "${CRATES[@]}"; do
  name="${entry%%|*}"; url="${entry##*|}"
  echo ">>> $name" >&2
  d="$WORK/$name"
  rm -rf "$d"
  git clone --depth 1 "$url" "$d" >/dev/null 2>&1 || { echo "clone failed: $name" >&2; continue; }
  src="$d/src"
  [ -d "$src" ] || { echo "no src/: $name" >&2; continue; }

  fb=$(nfiles "$src"); lb=$(loc "$src")
  read -r tb_status tb_n < <(runtests "$d")

  toollog=$("$TOOL" --recursive "$src" --no-fmt 2>&1)
  nsplit=$(echo "$toollog" | grep -c "✓")
  nroll=$(echo "$toollog" | grep -c "✗")

  fa=$(nfiles "$src"); la=$(loc "$src")
  read -r ta_status ta_n < <(runtests "$d")

  avgb=$(( fb>0 ? lb/fb : 0 ))
  avga=$(( fa>0 ? la/fa : 0 ))

  printf '| %s | %s | %s | %s | %s | %s | %s | %s | %s %s | %s %s |\n' \
    "$name" "$fb" "$fa" "$lb" "$avgb" "$avga" "$nsplit" "$nroll" "$tb_status" "$tb_n" "$ta_status" "$ta_n" >> "$OUT"
  echo "    $name: files $fb->$fa, loc $lb, split $nsplit roll $nroll, tests ${tb_status}${tb_n} -> ${ta_status}${ta_n}" >&2
done

echo "DONE — results written to $OUT" >&2
cat "$OUT"
