#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

WASM_MERGE="cargo run --manifest-path wasm-merge/Cargo.toml --release --quiet --"
IN=input/accessor
OUT=output
mkdir -p "$OUT"

# Helper: compile, merge, optimize, extract named functions
run_pipeline() {
  local label="$1" a_wat="$2" b_wat="$3" prefix="$4"
  shift 4
  local funcs=("$@")

  echo "### $label"
  echo ""
  echo "#### Module A"
  echo '```wat'
  cat "$IN/$a_wat"
  echo '```'
  echo ""
  echo "#### Module B"
  echo '```wat'
  cat "$IN/$b_wat"
  echo '```'
  echo ""

  wasm-tools parse "$IN/$a_wat" -o "$OUT/${prefix}_a.wasm"
  wasm-tools parse "$IN/$b_wat" -o "$OUT/${prefix}_b.wasm"

  $WASM_MERGE "$OUT/${prefix}_b.wasm=b" "$OUT/${prefix}_a.wasm=a" -o "$OUT/${prefix}_merged.wasm"
  echo "- merged: \`$(wc -c < "$OUT/${prefix}_merged.wasm")\` bytes"

  wasm-tools print "$OUT/${prefix}_merged.wasm" -o "$OUT/${prefix}_merged.wat"

  echo ""
  echo "#### After \`wasm-merge\`"
  echo '```wat'
  cat "$OUT/${prefix}_merged.wat"
  echo '```'
  echo ""

  if command -v wasm-opt &>/dev/null; then
    wasm-opt -O3 --inlining -O3 --enable-multimemory "$OUT/${prefix}_merged.wasm" -o "$OUT/${prefix}_optimized.wasm"
    wasm-tools print "$OUT/${prefix}_optimized.wasm" -o "$OUT/${prefix}_optimized.wat"

    echo "#### After \`wasm-opt -O3 --inlining -O3\`"
    echo ""
    if [ ${#funcs[@]} -gt 0 ]; then
      for func_name in "${funcs[@]}"; do
        fidx=$(grep "export \"${func_name}\"" "$OUT/${prefix}_optimized.wat" | grep -o 'func [0-9]*' | grep -o '[0-9]*')
        body=""
        if [ -n "$fidx" ]; then
          body=$(sed -n "/^  (func (;${fidx};)/,/^  )/p" "$OUT/${prefix}_optimized.wat")
        fi
        if [ -n "$body" ]; then
          echo "**\`${func_name}\`:**"
          echo '```wat'
          echo "$body"
          echo '```'
          echo ""
        fi
      done
    else
      echo '```wat'
      cat "$OUT/${prefix}_optimized.wat"
      echo '```'
      echo ""
    fi
  else
    echo "> wasm-opt not found — skipping optimization"
  fi
  echo "---"
  echo ""
}

{

cat <<'HEADER'
# Shared Memory Accessor Patterns

Module A owns a linear memory region. Module B accesses it only through
exported accessor functions — never via a raw pointer. After merge and
optimization, accessor calls are **completely eliminated** for provably-safe
accesses — zero-cost abstraction.

Two dimensions are demonstrated:
1. **Security** — how the access boundary is enforced (none, bounds check, opaque handle)
2. **Ownership** — who can read, write, or transfer the region (borrow, mutable borrow, move)

All patterns work today with multi-memory, `wasm-merge`, and `wasm-opt` — no spec changes needed.
HEADER

echo ""

# ══════════════════════════════════════════════���════════════════════════
echo "## Part 1: Security — accessor design comparison"
echo ""
echo "All three variants expose the string \`\"hello\"\` from Module A"
echo "to Module B. They differ in how the access boundary is enforced."
echo ""

run_pipeline \
  "1. INSECURE — raw i32 index, no bounds check" \
  insecure/a.wat insecure/b.wat insecure \
  read_first read_oob read_at_3 read_at

run_pipeline \
  "2. INLINE CHECK — bounds check inside accessor" \
  bounded/a.wat bounded/b.wat bounded \
  read_first read_oob read_at_3 read_at

run_pipeline \
  "3. TABLE INDIRECTION — funcref table + bounds check" \
  handle/a.wat handle/b.wat handle \
  read_first read_oob read_at_3 read_at

# Side-by-side comparison
if command -v wasm-opt &>/dev/null; then
  echo "### Security — side-by-side comparison"
  echo ""
  for func_name in read_first read_oob read_at_3 read_at; do
    echo "#### \`${func_name}\`"
    echo ""
    for entry in insecure:INSECURE bounded:"INLINE\ CHECK" handle:"TABLE\ INDIRECTION"; do
      name="${entry%%:*}"; label="${entry#*:}"
      label="${label//\\/}"
      fidx=$(grep "export \"${func_name}\"" "$OUT/${name}_optimized.wat" | grep -o 'func [0-9]*' | grep -o '[0-9]*')
      body=""
      if [ -n "$fidx" ]; then
        body=$(sed -n "/^  (func (;${fidx};)/,/^  )/p" "$OUT/${name}_optimized.wat")
      fi
      if [ -n "$body" ]; then
        echo "**${label}:**"
        echo '```wat'
        echo "$body"
        echo '```'
        echo ""
      fi
    done
  done
fi

cat <<'SEC_SUMMARY'
### Security summary

| Function | Index | Insecure | Inline check | Table indirection |
|----------|-------|----------|--------------|-------------------|
| `read_first()` | `0` (static) | direct load | check eliminated (0 < 5) | check eliminated |
| `read_oob()` | `10` (static) | direct load (reads garbage) | reduced to `unreachable` (10 >= 5) | reduced to `unreachable` |
| `read_at_3()` | `3` (static) | direct load | check eliminated (3 < 5) | check eliminated |
| `read_at(i)` | dynamic | direct load | **bounds check preserved** | **bounds check preserved** |

For static indices, the optimizer proves safety at compile time. For dynamic
indices, the bounds check survives — the runtime safety net.

SEC_SUMMARY

# ════════════════════════════════════════════════��══════════════════════
echo "## Part 2: Ownership — borrow and move semantics"
echo ""
echo "Three patterns showing how Rust-inspired ownership semantics can be"
echo "enforced at the module boundary using accessor functions."
echo ""

run_pipeline \
  "4. READ-ONLY BORROW — read accessor only, no write" \
  readonly/a.wat readonly/b.wat readonly \
  count_vowels

run_pipeline \
  "5. MUTABLE BORROW — read + write accessors, call-scoped" \
  mutborrow/a.wat mutborrow/b.wat mutborrow \
  uppercase read_back

run_pipeline \
  "6. MOVE — read + release, use-after-move traps" \
  move/a.wat move/b.wat move \
  take_data read_after_move

cat <<'OWN_SUMMARY'
### Ownership summary

| Function | Pattern | After optimization |
|---|---|---|
| `count_vowels()` | read-only borrow | accessor inlined → direct `i32.load8_u` from A's memory |
| `uppercase()` | mutable borrow | read+write inlined → direct `i32.load8_u` + `i32.store8` on A's memory |
| `read_back(i)` | mutable borrow | accessor inlined → direct `i32.load8_u` |
| `take_data()` | move | read inlined, `$alive` check + release preserved |
| `read_after_move()` | move | `$alive` check preserved — **traps at runtime** |

**Read-only borrow**: No write accessor exists — security is structural.

**Mutable borrow (call-scoped)**: Synchronous single-threaded Wasm guarantees
mutual exclusion. After optimization, read+write inline to direct memory ops.

**Move / transfer**: After `release()`, A's `$alive` flag traps all subsequent
accesses. The optimizer correctly preserves the check — it cannot prove the
flag's value statically.

OWN_SUMMARY

# ════════════════════════════════════════════════════��══════════════════
cat <<'FOOTER'
## Mapping to source languages

| Pattern | Rust | C++ | C |
|---|---|---|---|
| Read-only borrow | `&str` | `string_view` | `const char*` |
| Mutable borrow | `&mut [u8]` | `span<char>` | `char*` (by convention) |
| Move (call-scoped) | `fn(Vec<u8>) -> Vec<u8>` | `vector<uint8_t>&&` + return | annotated macro |
| Move (permanent) | `fn(Vec<u8>)` | `vector<uint8_t>&&` | annotated macro |

## Implications

Component-style shared-nothing linking could become the **default inside a
component** — not just between components. With multi-memory and accessor
functions as the default linking strategy, isolation would be the default
and shared access would be the explicit opt-in, raising the baseline security
of statically linked Wasm without requiring any changes to the Component Model.
FOOTER

} > docs/ACCESSOR.md 2>/dev/null

echo "Wrote docs/ACCESSOR.md"
