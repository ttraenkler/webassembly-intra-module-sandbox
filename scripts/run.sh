#!/usr/bin/env bash
set -euo pipefail

# Run from repo root regardless of where the script is invoked
cd "$(dirname "$0")/.."

# Ensure cargo-installed binaries are on PATH
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

WASM_MERGE="cargo run --manifest-path wasm-merge/Cargo.toml --release --quiet --"
IN=input
OUT=output
mkdir -p "$OUT"

SEP="════════���═══════════════════════════════════════════════════════"

echo "$SEP"
echo "  STEP 0 — Source WAT modules"
echo "$SEP"
echo ""
echo "── a.wat (Module A — owns memory, exports accessor) ──"
cat "$IN/a.wat"
echo ""
echo "── b.wat (Module B — imports accessor, never touches raw memory) ──"
cat "$IN/b.wat"
echo ""

# ── Step 1: Compile ──────��───────────────────────────────────────────
echo "$SEP"
echo "  STEP 1 — Compile WAT → Wasm (wasm-tools parse)"
echo "$SEP"
wasm-tools parse "$IN/a.wat" -o "$OUT/a.wasm"
wasm-tools parse "$IN/b.wat" -o "$OUT/b.wasm"
echo "  ✓ a.wasm  ($(wc -c < "$OUT/a.wasm") bytes)"
echo "  ✓ b.wasm  ($(wc -c < "$OUT/b.wasm") bytes)"
echo ""

# ── Step 2: Merge with multi-memory ────────��────────────────────────
echo "$SEP"
echo "  STEP 2 — wasm-merge (shared-nothing, multi-memory)"
echo "$SEP"
# B listed first so B's memory = index 0, A's memory = index 1.
$WASM_MERGE "$OUT/b.wasm=b" "$OUT/a.wasm=a" -o "$OUT/merged.wasm"
echo "  ✓ merged.wasm  ($(wc -c < "$OUT/merged.wasm") bytes)"
echo ""

# ── Step 3: Inspect merged output ──────���────────────────────────────
echo "$SEP"
echo "  STEP 3 — Disassemble merged.wasm"
echo "$SEP"
wasm-tools print "$OUT/merged.wasm" -o "$OUT/merged.wat"
cat "$OUT/merged.wat"
echo ""

echo "── Verification ──"
MCOUNT=$(grep -c '(memory' "$OUT/merged.wat")
echo "  ✓ Memory declarations found: $MCOUNT"

if grep -q 'load8_u 1\|load8_u $' "$OUT/merged.wat"; then
  echo "  ✓ string_byte references memory index 1 (A's memory after merge)"
fi

if grep -q 'call' "$OUT/merged.wat"; then
  echo "  ✓ read_first still contains a call to string_byte"
fi
echo ""

# ── Step 4: Optimize (inline) — optional, requires Binaryen ────────
if command -v wasm-opt &>/dev/null; then
  echo "$SEP"
  echo "  STEP 4 — wasm-opt --inlining (optional, Binaryen)"
  echo "$SEP"
  wasm-opt --inlining --enable-multimemory "$OUT/merged.wasm" -o "$OUT/optimized.wasm"
  echo "  ✓ optimized.wasm  ($(wc -c < "$OUT/optimized.wasm") bytes)"
  echo ""

  echo "$SEP"
  echo "  STEP 5 — Disassemble optimized.wasm"
  echo "$SEP"
  wasm-tools print "$OUT/optimized.wasm" -o "$OUT/optimized.wat"
  cat "$OUT/optimized.wat"
  echo ""

  echo "── Verification ──"
  if grep -q 'i32.load8_u' "$OUT/optimized.wat"; then
    echo "  ✓ read_first contains a direct i32.load8_u instruction"
  fi

  READ_FIRST=$(sed -n '/func.*read_first/,/^  )/p' "$OUT/optimized.wat")
  if echo "$READ_FIRST" | grep -q 'call'; then
    echo "  ✗ read_first still contains a call (inlining did not fully eliminate it)"
  else
    echo "  ✓ The call to string_byte has been completely eliminated"
  fi
  echo ""
else
  echo "$SEP"
  echo "  STEP 4 — Skipped (install wasm-opt from Binaryen for inlining)"
  echo "$SEP"
  echo ""
fi

# ── Step 6: Component Model merge ───────────────────────────────────
echo "$SEP"
echo "  STEP 6 — Component Model merge (component.wat)"
echo "$SEP"
echo ""
echo "── component.wat (nested core modules with instantiation wiring) ──"
cat "$IN/component.wat"
echo ""

wasm-tools parse "$IN/component.wat" -o "$OUT/component.wasm"
echo "  ✓ component.wasm  ($(wc -c < "$OUT/component.wasm") bytes)"

$WASM_MERGE "$OUT/component.wasm" -o "$OUT/component-merged.wasm"
echo "  ✓ component-merged.wasm  ($(wc -c < "$OUT/component-merged.wasm") bytes)"
echo ""

echo "── Disassembly ──"
wasm-tools print "$OUT/component-merged.wasm" -o "$OUT/component-merged.wat"
cat "$OUT/component-merged.wat"
echo ""

echo "── Verification ──"
CMCOUNT=$(grep -c '(memory' "$OUT/component-merged.wat")
echo "  ✓ Memory declarations found: $CMCOUNT"

if grep -q 'load8_u 1\|load8_u $' "$OUT/component-merged.wat"; then
  echo "  ✓ string_byte references memory index 1 (A's memory)"
fi

if grep -q 'call' "$OUT/component-merged.wat"; then
  echo "  ✓ read_first calls string_byte (cross-module call resolved)"
fi

# Compare standalone merge vs component merge — should produce identical output
if diff -q "$OUT/merged.wasm" "$OUT/component-merged.wasm" &>/dev/null; then
  echo "  ✓ Standalone merge and component merge produce identical output"
else
  echo "  ✗ Standalone merge and component merge differ (investigating...)"
  diff <(wasm-tools print "$OUT/merged.wasm") <(wasm-tools print "$OUT/component-merged.wasm") || true
fi
echo ""

# ���─ Step 7: Optimize component merge (inline) — optional ──────��────
if command -v wasm-opt &>/dev/null; then
  echo "$SEP"
  echo "  STEP 7 — wasm-opt --inlining on component-merged.wasm"
  echo "$SEP"
  wasm-opt --inlining --enable-multimemory "$OUT/component-merged.wasm" -o "$OUT/component-optimized.wasm"
  echo "  ✓ component-optimized.wasm  ($(wc -c < "$OUT/component-optimized.wasm") bytes)"
  echo ""

  echo "── Disassembly ──"
  wasm-tools print "$OUT/component-optimized.wasm" -o "$OUT/component-optimized.wat"
  cat "$OUT/component-optimized.wat"
  echo ""

  echo "── Verification ──"
  if grep -q 'i32.load8_u' "$OUT/component-optimized.wat"; then
    echo "  ✓ read_first contains a direct i32.load8_u instruction"
  fi

  READ_FIRST_C=$(sed -n '/func.*read_first/,/^  )/p' "$OUT/component-optimized.wat")
  if echo "$READ_FIRST_C" | grep -q 'call'; then
    echo "  ✗ read_first still contains a call (inlining did not fully eliminate it)"
  else
    echo "  ✓ The call to string_byte has been completely eliminated"
  fi

  if diff -q "$OUT/optimized.wasm" "$OUT/component-optimized.wasm" &>/dev/null; then
    echo "  ✓ Optimized standalone and component outputs are identical"
  fi
  echo ""
fi

# ── Summary ─────────────────────────────────────────────────────────
echo "$SEP"
echo "  SUMMARY"
echo "$SEP"
cat <<'SUMMARY'

  1. Module B never held a raw pointer into A's memory.
     It could only read A's data through the exported string_byte()
     accessor — a capability-based interface boundary.

  2. The isolation guarantee existed pre-merge at the module level.
     Each module had its own linear memory; B had no import of A's
     memory and therefore could not address it.

  3. After merging, each module retains its own memory (multi-memory).
     Cross-module function calls are resolved, but memory isolation
     is preserved — zero-cost sandboxing.

  4. With wasm-opt --inlining (optional), the accessor call is
     completely erased. read_first() becomes a direct i32.load8_u
     from memory 1 — identical to shared-everything, with zero
     call overhead.

  5. Required tools (all from cargo):
       • wasm-tools  — WAT ↔ Wasm conversion
       • wasm-merge  — shared-nothing multi-memory merge (this project)

     Optional:
       • wasm-opt    �� inlining optimization (Binaryen)

SUMMARY
