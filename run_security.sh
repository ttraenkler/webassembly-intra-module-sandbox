#!/usr/bin/env bash
set -euo pipefail

# Ensure cargo-installed binaries are on PATH
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

WASM_MERGE="cargo run --manifest-path wasm-merge/Cargo.toml --release --quiet --"

# Helper: compile, merge, optimize, show full modules
run_pipeline() {
  local label="$1" a_wat="$2" b_wat="$3" prefix="$4"

  echo "### $label"
  echo ""

  echo "#### Module A"
  echo ""
  echo '```wat'
  cat "$a_wat"
  echo '```'
  echo ""
  echo "#### Module B"
  echo ""
  echo '```wat'
  cat "$b_wat"
  echo '```'
  echo ""

  # Compile
  wasm-tools parse "$a_wat" -o "${prefix}_a.wasm"
  wasm-tools parse "$b_wat" -o "${prefix}_b.wasm"
  echo "- compiled: \`$(wc -c < "${prefix}_a.wasm")\` + \`$(wc -c < "${prefix}_b.wasm")\` bytes"

  # Merge
  $WASM_MERGE "${prefix}_b.wasm=b" "${prefix}_a.wasm=a" -o "${prefix}_merged.wasm"
  echo "- merged: \`$(wc -c < "${prefix}_merged.wasm")\` bytes"

  # Disassemble merged
  wasm-tools print "${prefix}_merged.wasm" -o "${prefix}_merged.wat"

  echo ""
  echo "#### After \`wasm-merge\`"
  echo ""
  echo '```wat'
  cat "${prefix}_merged.wat"
  echo '```'
  echo ""

  # Optimize (if wasm-opt available)
  if command -v wasm-opt &>/dev/null; then
    wasm-opt -O3 --inlining -O3 --enable-multimemory "${prefix}_merged.wasm" -o "${prefix}_optimized.wasm"
    wasm-tools print "${prefix}_optimized.wasm" -o "${prefix}_optimized.wat"

    echo "#### After \`wasm-opt -O3 --inlining -O3\`"
    echo ""
    echo '```wat'
    cat "${prefix}_optimized.wat"
    echo '```'
    echo ""

    # Extract individual functions for clarity
    echo "#### Optimized functions"
    echo ""
    for func_name in read_first read_oob read_at_3 read_at; do
      fidx=$(grep "export \"${func_name}\"" "${prefix}_optimized.wat" | grep -o 'func [0-9]*' | grep -o '[0-9]*')
      body=""
      if [ -n "$fidx" ]; then
        body=$(sed -n "/^  (func (;${fidx};)/,/^  )/p" "${prefix}_optimized.wat")
      fi
      if [ -n "$body" ]; then
        echo "**\`${func_name}\`:**"
        echo ""
        echo '```wat'
        echo "$body"
        echo '```'
        echo ""
      fi
    done
  else
    echo "> wasm-opt not found — skipping optimization"
  fi
  echo ""
  echo "---"
  echo ""
}

{

echo "# Security Comparison: three approaches to accessor design"
echo ""
echo "All three modules expose the string \`\"hello\"\` from Module A"
echo "to Module B. They differ in how the access boundary is enforced."
echo ""

# ── 1. Insecure (original) ─────────────────────────────────────
run_pipeline \
  "1. INSECURE — raw i32 index, no bounds check" \
  a.wat b.wat insecure

# ── 2. Bounds-checked ──────────────────────────────────────────
run_pipeline \
  "2. INLINE CHECK — bounds check inside accessor" \
  a_bounded.wat b_bounded.wat bounded

# ── 3. Table indirection ──────────────────────────────────────
run_pipeline \
  "3. TABLE INDIRECTION — funcref table + bounds check" \
  a_handle.wat b_handle.wat handle

# ── Side-by-side comparison ────────────────────────────────────
echo "## Comparison — optimized functions across all three approaches"
echo ""

if command -v wasm-opt &>/dev/null; then
  for func_name in read_first read_oob read_at_3 read_at; do
    echo "### \`${func_name}\`"
    echo ""
    for entry in insecure:INSECURE bounded:"INLINE\ CHECK" handle:"TABLE\ INDIRECTION"; do
      name="${entry%%:*}"; label="${entry#*:}"
      label="${label//\\/}"
      # Find the function index for this export name, then extract that function body
      fidx=$(grep "export \"${func_name}\"" "${name}_optimized.wat" | grep -o 'func [0-9]*' | grep -o '[0-9]*')
      body=""
      if [ -n "$fidx" ]; then
        body=$(sed -n "/^  (func (;${fidx};)/,/^  )/p" "${name}_optimized.wat")
      fi
      if [ -n "$body" ]; then
        echo "**${label}:**"
        echo ""
        echo '```wat'
        echo "$body"
        echo '```'
        echo ""
      fi
    done
  done
else
  echo "> Install wasm-opt for optimization comparison."
  echo ""
fi

# ── Summary ────────────────────────────────────────────────────
cat <<'SUMMARY'
## Summary

**INSECURE:** `string_byte(i)` loads any byte from A's memory — B can pass
any i32 value and read A's entire address space.

**INLINE CHECK:** A knows the string bounds internally (`base=0`, `len=5`) and
traps on out-of-bounds access. B's call site is unchanged, but the security
boundary is enforced within A. The bounds check is visible in the exported
accessor (`i32.ge_u` + `unreachable`) and survives optimization for dynamic
callers.

**TABLE INDIRECTION:** B receives an opaque handle that indexes into a Wasm
`funcref` table. Each table slot holds a bounds-checked accessor for a specific
memory region. `get_byte(handle, i)` dispatches via `call_indirect` — the Wasm
runtime bounds-checks the handle automatically, and each accessor bounds-checks
the byte index. B cannot read the table, forge entries, or call arbitrary
addresses. The table is a first-class Wasm construct, not a data structure in
linear memory.

### What the optimizer does

| Function | Index | Insecure | Inline check | Table indirection |
|----------|-------|----------|--------------|-------------------|
| `read_first()` | `0` (static) | direct load | check eliminated (0 < 5) | check eliminated |
| `read_oob()` | `10` (static) | direct load (reads garbage) | reduced to `unreachable` (10 >= 5) | reduced to `unreachable` |
| `read_at_3()` | `3` (static) | direct load | check eliminated (3 < 5) | check eliminated |
| `read_at(i)` | dynamic | direct load | **bounds check preserved** | **bounds check preserved** |

For static indices, the optimizer proves safety (or unsafety) at compile time.
For dynamic indices, the bounds check must survive — it's the runtime safety net.

The insecure version produces a bare `i32.load8_u` in all cases — fast, but
no safety at any level.

**Zero-cost abstraction: safety at the source level, bare metal at runtime
for provably-safe accesses, minimal overhead for dynamic ones.**
SUMMARY

} > SECURITY.md 2>/dev/null

echo "Wrote SECURITY.md"
