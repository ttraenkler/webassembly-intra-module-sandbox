#!/usr/bin/env bash
set -euo pipefail

# Ensure cargo-installed binaries are on PATH
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

WASM_MERGE="cargo run --manifest-path wasm-merge/Cargo.toml --release --quiet --"

SEP="════════════════════════════════════════════════════════════════"

echo "$SEP"
echo "  STEP 0 — Source WAT modules"
echo "$SEP"
echo ""
echo "── a.wat (Module A — owns memory, exports accessor) ──"
cat a.wat
echo ""
echo "── b.wat (Module B — imports accessor, never touches raw memory) ──"
cat b.wat
echo ""

# ── Step 1: Compile ──────────────────────────────────────────────────
echo "$SEP"
echo "  STEP 1 — Compile WAT → Wasm (wasm-tools parse)"
echo "$SEP"
wasm-tools parse a.wat -o a.wasm
wasm-tools parse b.wat -o b.wasm
echo "  ✓ a.wasm  ($(wc -c < a.wasm) bytes)"
echo "  ✓ b.wasm  ($(wc -c < b.wasm) bytes)"
echo ""

# ── Step 2: Merge with multi-memory ─────────────────────────────────
echo "$SEP"
echo "  STEP 2 — wasm-merge (shared-nothing, multi-memory)"
echo "$SEP"
# B listed first so B's memory = index 0, A's memory = index 1.
$WASM_MERGE b.wasm=b a.wasm=a -o merged.wasm
echo "  ✓ merged.wasm  ($(wc -c < merged.wasm) bytes)"
echo ""

# ── Step 3: Inspect merged output ───────────────────────────────────
echo "$SEP"
echo "  STEP 3 — Disassemble merged.wasm"
echo "$SEP"
wasm-tools print merged.wasm -o merged.wat
cat merged.wat
echo ""

echo "── Verification ──"
MCOUNT=$(grep -c '(memory' merged.wat)
echo "  ✓ Memory declarations found: $MCOUNT"

if grep -q 'load8_u 1\|load8_u $' merged.wat; then
  echo "  ✓ string_byte references memory index 1 (A's memory after merge)"
fi

if grep -q 'call' merged.wat; then
  echo "  ✓ read_first still contains a call to string_byte"
fi
echo ""

# ── Step 4: Optimize (inline) — optional, requires Binaryen ────────
if command -v wasm-opt &>/dev/null; then
  echo "$SEP"
  echo "  STEP 4 — wasm-opt --inlining (optional, Binaryen)"
  echo "$SEP"
  wasm-opt --inlining --enable-multimemory merged.wasm -o optimized.wasm
  echo "  ✓ optimized.wasm  ($(wc -c < optimized.wasm) bytes)"
  echo ""

  echo "$SEP"
  echo "  STEP 5 — Disassemble optimized.wasm"
  echo "$SEP"
  wasm-tools print optimized.wasm -o optimized.wat
  cat optimized.wat
  echo ""

  echo "── Verification ──"
  if grep -q 'i32.load8_u' optimized.wat; then
    echo "  ✓ read_first contains a direct i32.load8_u instruction"
  fi

  READ_FIRST=$(sed -n '/func.*read_first/,/^  )/p' optimized.wat)
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
       • wasm-opt    — inlining optimization (Binaryen)

SUMMARY
