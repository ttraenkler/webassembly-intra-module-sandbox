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

# ════════════════════════════════════════════════════════════════════════
echo "## Part 3: Primitives and structs — trivial zero-cost patterns"
echo ""
echo "When the cross-module interface passes primitive values (i32, i64, f32, f64)"
echo "or struct fields via individual accessors, values travel on the Wasm stack —"
echo "no memory copy, no bounds check needed."
echo ""

run_pipeline \
  "7. PRIMITIVE — counter via global, values on the stack" \
  primitive/a.wat primitive/b.wat primitive \
  inc_and_get inc_n

run_pipeline \
  "8. STRUCT — Point {x, y} via field accessors" \
  struct/a.wat struct/b.wat struct \
  distance_squared translate

cat <<'PRIM_SUMMARY'
### Primitives and structs summary

| Function | Pattern | After optimization |
|---|---|---|
| `inc_and_get()` | primitive (global) | direct `global.set` + `global.get` — zero call overhead |
| `inc_n(n)` | primitive (loop) | inlined increment loop — no cross-module call per iteration |
| `distance_squared()` | struct field accessors | direct `i32.load` from A's memory — accessors eliminated |
| `translate(dx, dy)` | struct field accessors | direct `i32.load` + `i32.store` on A's memory |

**Primitives**: values pass on the Wasm stack. No memory involved, no accessor
needed for the return value. After inlining, the call disappears entirely —
`inc_and_get()` becomes a direct global increment and read.

**Structs via field accessors**: each field has its own getter/setter. After
inlining, these become direct `i32.load`/`i32.store` on A's memory at fixed
offsets — identical to accessing a struct in shared memory, but with isolation
preserved.

PRIM_SUMMARY

# ════════════════════════════════════════════════════════════════════════
echo "## Part 4: GC types — type-system enforced isolation"
echo ""
echo "With the Wasm GC proposal, structs and arrays live on the runtime-managed"
echo "GC heap — not in linear memory. References pass on the stack, and the type"
echo "system enforces field access. No accessor functions or bounds checks needed:"
echo "\`struct.get\`/\`struct.set\` are already single instructions."
echo ""
echo "> Note: \`wasm-merge\` does not yet handle GC types. These examples show the"
echo "> pattern — merging GC modules requires GC-aware tooling."
echo ""

echo "### 9. GC STRUCT — Point {x, y} on the GC heap"
echo ""
echo "#### Module A"
echo '```wat'
cat "$IN/gc/a.wat"
echo '```'
echo ""
echo "#### Module B"
echo '```wat'
cat "$IN/gc/b.wat"
echo '```'
echo ""

cat <<'GC_SUMMARY'
### GC types summary

| Pattern | Linear memory accessor | GC struct |
|---|---|---|
| Create | allocate in memory, return offset | `struct.new` — runtime allocates |
| Read field | `get_x()` → `i32.load offset=0` | `struct.get $Point $x` |
| Write field | `set_x(v)` → `i32.store offset=0` | `struct.set $Point $x` |
| Safety | bounds check in accessor | type system — cannot access undeclared fields |
| After inlining | direct `i32.load`/`i32.store` | already a single instruction — nothing to inline |
| Forgery | B cannot forge a pointer (no raw address) | B cannot forge a reference (GC-managed) |

**Linear memory structs** need accessor functions to maintain isolation. After
merge + inline, these become direct loads/stores — zero-cost but requires tooling.

**GC structs** need no accessor functions at all. `struct.get`/`struct.set` are
already single instructions with type-system enforced field access. The isolation
is built into the instruction set. GC types are the natural choice for languages
targeting the GC proposal (Kotlin, Dart, Java, OCaml) while linear memory
accessors serve C/C++/Rust and existing wasi-libc-based toolchains.

GC_SUMMARY

# ════════════════════════════════════════════════════════════════════════
echo "## Part 5: Benchmarks — accessor overhead"
echo ""

# Build a benchmark that measures accessor call overhead
# Compare: separate modules (cross-module call) vs merged vs merged+optimized
cat > "$OUT/bench_accessor.wat" << 'EOF'
(module
  (import "a" "increment" (func $inc))
  (import "a" "get_counter" (func $get (result i32)))
  (memory (export "b_memory") 1)
  (func (export "bench") (param $n i32) (result i32)
    (local $i i32)
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (call $inc)
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (call $get))
)
EOF
wasm-tools parse "$OUT/bench_accessor.wat" -o "$OUT/bench_accessor.wasm"
wasm-tools parse "$IN/primitive/a.wat" -o "$OUT/primitive_a.wasm"

# Merged (calls preserved)
$WASM_MERGE "$OUT/bench_accessor.wasm=b" "$OUT/primitive_a.wasm=a" -o "$OUT/bench_accessor_merged.wasm"
# Merged + optimized (calls inlined)
if command -v wasm-opt &>/dev/null; then
  wasm-opt -O3 --inlining -O3 --enable-multimemory "$OUT/bench_accessor_merged.wasm" -o "$OUT/bench_accessor_opt.wasm"
fi

N_CALLS=5000000

echo "Measuring $N_CALLS increment calls on V8:"
echo ""
echo '```'
node --eval "
const { readFileSync } = require('fs');
const N = $N_CALLS;

// Separate modules (cross-module call)
const a_mod = new WebAssembly.Module(readFileSync('$OUT/primitive_a.wasm'));
const a_inst = new WebAssembly.Instance(a_mod);
const b_mod = new WebAssembly.Module(readFileSync('$OUT/bench_accessor.wasm'));
const b_inst = new WebAssembly.Instance(b_mod, { a: a_inst.exports });

// Merged (multi-memory, calls preserved)
const merged_mod = new WebAssembly.Module(readFileSync('$OUT/bench_accessor_merged.wasm'));
const merged_inst = new WebAssembly.Instance(merged_mod);

// Merged + optimized (calls inlined)
const opt_mod = new WebAssembly.Module(readFileSync('$OUT/bench_accessor_opt.wasm'));
const opt_inst = new WebAssembly.Instance(opt_mod);

const targets = [
  ['Separate (cross-module call)', b_inst],
  ['Merged (call preserved)',      merged_inst],
  ['Merged + optimized (inlined)', opt_inst],
];

// Warmup
for (const [, inst] of targets) for (let i = 0; i < 50000; i++) inst.exports.bench(1);

const runs = 5;
const results = targets.map(() => []);
for (let r = 0; r < runs; r++) {
  for (let t = 0; t < targets.length; t++) {
    const t0 = performance.now();
    targets[t][1].exports.bench(N);
    results[t].push(performance.now() - t0);
  }
}

const base = results[0].sort((a,b)=>a-b)[Math.floor(runs/2)];
for (let t = 0; t < targets.length; t++) {
  const med = results[t].sort((a,b)=>a-b)[Math.floor(runs/2)];
  const pct = t === 0 ? '' : ' (' + ((med / base - 1) * 100).toFixed(0) + '%)';
  console.log(targets[t][0].padEnd(40) + med.toFixed(1).padStart(8) + ' ms' + pct);
}
" 2>&1
echo '```'
echo ""

cat <<'BENCH_SUMMARY'
### Benchmark observations

- **Separate modules**: each call crosses the module boundary — the runtime
  cannot inline across modules (V8 compiles at the module level).
- **Merged (call preserved)**: both functions are in the same module but the
  call instruction remains. V8 can now inline at JIT time.
- **Merged + optimized**: `wasm-opt` inlines the call ahead of time. The loop
  body is a direct `global.set` + `global.get` — no call at all.

The progression from separate → merged → optimized shows the full zero-cost
path: module boundary eliminated by merge, call overhead eliminated by inlining.

BENCH_SUMMARY

# ════════════════════════════════════════════════════════════════════════
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
