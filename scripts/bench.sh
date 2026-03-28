#!/usr/bin/env bash
set -euo pipefail

# Run from repo root regardless of where the script is invoked
cd "$(dirname "$0")/.."

export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

# bench.sh — build all variants, run benchmarks on V8 and Wasmtime,
# write results to output/bench.json
#
# Prerequisites: wasm-tools, wasm-opt, wasmtime, node
# The real wasi-libc module at /tmp/real_lib.wasm (built by wasi-sdk)

MERGE="cargo run --manifest-path wasm-merge/Cargo.toml --release --quiet --"
OUT=output
mkdir -p "$OUT"

WASI_LIB=input/wasi-libc.wasm
if [ ! -f "$WASI_LIB" ]; then
  echo "Missing $WASI_LIB — compile with wasi-sdk first"
  exit 1
fi

Ns=(2 5 10 20 50 100)

echo "=== Step 1: Build baseline (single consumer + unmodified lib) ==="

cat > "$OUT/consumer_baseline.wat" << 'EOF'
(module
  (import "lib" "malloc" (func $malloc (param i32) (result i32)))
  (import "lib" "free" (func $free (param i32)))
  (memory (export "memory") 4)
  (func (export "run") (param $size i32) (result i32)
    (call $malloc (local.get $size))))
EOF
wasm-tools parse "$OUT/consumer_baseline.wat" -o "$OUT/consumer_baseline.wasm"
$MERGE "$OUT/consumer_baseline.wasm=lib" "$WASI_LIB=lib" -o "$OUT/baseline.wasm" 2>/dev/null
echo "  baseline: $(wc -c < "$OUT/baseline.wasm") bytes"

echo ""
echo "=== Step 2: Build shared variants for each N ==="

# Consumer template: imports malloc+free with ORIGINAL names (no __instN)
gen_consumer() {
  local n=$1 i=$2
  cat > "$OUT/c${n}_${i}.wat" << EOFWAT
(module
  (import "lib" "malloc" (func \$malloc (param i32) (result i32)))
  (import "lib" "free" (func \$free (param i32)))
  (memory (export "memory") 4)
  (func (export "run_${i}") (param \$size i32) (result i32)
    (call \$malloc (local.get \$size))))
EOFWAT
  wasm-tools parse "$OUT/c${n}_${i}.wat" -o "$OUT/c${n}_${i}.wasm"
}

for n in "${Ns[@]}"; do
  # Generate N consumers
  for i in $(seq 0 $((n-1))); do gen_consumer "$n" "$i"; done

  # Dispatch mode (integrated in wasm-merge --dispatch)
  echo -n "  N=$n dispatch..."
  ARGS=""
  for i in $(seq 0 $((n-1))); do ARGS="$ARGS $OUT/c${n}_${i}.wasm=inst${i}"; done
  ARGS="$ARGS $WASI_LIB=lib"
  $MERGE $ARGS --dispatch --lib lib -o "$OUT/shared_n${n}.wasm" 2>/dev/null
  wasm-opt -O4 --enable-multimemory --enable-bulk-memory "$OUT/shared_n${n}.wasm" -o "$OUT/shared_n${n}_O4.wasm" 2>/dev/null
  echo " O4=$(wc -c < "$OUT/shared_n${n}_O4.wasm")"

  # Specialize mode (integrated in wasm-merge --specialize)
  echo -n "  N=$n specialize..."
  ARGS=""
  for i in $(seq 0 $((n-1))); do ARGS="$ARGS $OUT/c${n}_${i}.wasm=inst${i}"; done
  ARGS="$ARGS $WASI_LIB=lib"
  $MERGE $ARGS --specialize --lib lib -o "$OUT/spec_n${n}.wasm" 2>/dev/null
  wasm-opt --remove-unused-module-elements --enable-multimemory --enable-bulk-memory \
    "$OUT/spec_n${n}.wasm" -o "$OUT/spec_n${n}_dce.wasm" 2>/dev/null || true
  echo " dce=$(wc -c < "$OUT/spec_n${n}_dce.wasm" 2>/dev/null || echo 0)"
done

echo ""
echo "=== Step 3: Build self-benchmarking Wasm modules ==="

# Benchmark harness WAT — imports malloc+free, pure Wasm timing loop
cat > "$OUT/bench_harness.wat" << 'BENCHEOF'
(module
  (import "wasi_snapshot_preview1" "clock_time_get" (func $clock (param i32 i64 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "lib" "malloc" (func $malloc (param i32) (result i32)))
  (import "lib" "free" (func $free (param i32)))
  (memory (export "memory") 4)
  (func $now (result i64)
    (drop (call $clock (i32.const 1) (i64.const 0) (i32.const 200)))
    (i64.load (i32.const 200)))
  (func $itoa (param $val i64) (param $buf i32) (result i32)
    (local $len i32) (local $tmp i64) (local $i i32) (local $j i32) (local $c i32)
    (if (i64.eqz (local.get $val)) (then (i32.store8 (local.get $buf) (i32.const 48)) (return (i32.const 1))))
    (local.set $tmp (local.get $val))
    (block $b (loop $l (br_if $b (i64.eqz (local.get $tmp)))
      (i32.store8 (i32.add (local.get $buf) (local.get $len)) (i32.add (i32.const 48) (i32.wrap_i64 (i64.rem_u (local.get $tmp) (i64.const 10)))))
      (local.set $tmp (i64.div_u (local.get $tmp) (i64.const 10)))
      (local.set $len (i32.add (local.get $len) (i32.const 1))) (br $l)))
    (local.set $i (i32.const 0)) (local.set $j (i32.sub (local.get $len) (i32.const 1)))
    (block $rb (loop $rl (br_if $rb (i32.ge_u (local.get $i) (local.get $j)))
      (local.set $c (i32.load8_u (i32.add (local.get $buf) (local.get $i))))
      (i32.store8 (i32.add (local.get $buf) (local.get $i)) (i32.load8_u (i32.add (local.get $buf) (local.get $j))))
      (i32.store8 (i32.add (local.get $buf) (local.get $j)) (local.get $c))
      (local.set $i (i32.add (local.get $i) (i32.const 1))) (local.set $j (i32.sub (local.get $j) (i32.const 1))) (br $rl)))
    (local.get $len))
  (func $print (param $ns i64)
    (local $len i32)
    (local.set $len (call $itoa (i64.div_u (local.get $ns) (i64.const 1000)) (i32.const 500)))
    (i32.store (i32.const 300) (i32.const 500)) (i32.store (i32.const 304) (local.get $len))
    (drop (call $fd_write (i32.const 1) (i32.const 300) (i32.const 1) (i32.const 400)))
    (i32.store8 (i32.const 600) (i32.const 10))
    (i32.store (i32.const 300) (i32.const 600)) (i32.store (i32.const 304) (i32.const 1))
    (drop (call $fd_write (i32.const 1) (i32.const 300) (i32.const 1) (i32.const 400))))
  (func (export "_start")
    (local $i i32) (local $t0 i64) (local $t1 i64)
    ;; warmup: 5K malloc+free
    (local.set $i (i32.const 0))
    (block $wb (loop $wl (br_if $wb (i32.ge_u (local.get $i) (i32.const 5000)))
      (call $free (call $malloc (i32.const 8)))
      (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $wl)))
    ;; measure: 50K malloc+free
    (local.set $t0 (call $now))
    (local.set $i (i32.const 0))
    (block $b (loop $l (br_if $b (i32.ge_u (local.get $i) (i32.const 50000)))
      (call $free (call $malloc (i32.const 8)))
      (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $l)))
    (local.set $t1 (call $now))
    (call $print (i64.sub (local.get $t1) (local.get $t0))))
)
BENCHEOF
wasm-tools parse "$OUT/bench_harness.wat" -o "$OUT/bench_harness.wasm"

# (dispatch mode now uses original import names, no __instN harness needed)

# Baseline bench module
$MERGE "$OUT/bench_harness.wasm=lib" "$WASI_LIB=lib" -o "$OUT/bench_baseline.wasm" 2>/dev/null
echo "  bench_baseline: $(wc -c < "$OUT/bench_baseline.wasm") bytes"

# Per-N bench modules
for n in "${Ns[@]}"; do
  # Dispatch bench (uses original import names — bench_harness imports "lib" "malloc")
  ARGS="$OUT/bench_harness.wasm=inst0"
  for i in $(seq 1 $((n-1))); do ARGS="$ARGS $OUT/c${n}_${i}.wasm=inst${i}"; done
  ARGS="$ARGS $WASI_LIB=lib"
  $MERGE $ARGS --dispatch --lib lib -o "$OUT/bench_shared_n${n}.wasm" 2>/dev/null
  wasm-opt -O4 --enable-multimemory --enable-bulk-memory "$OUT/bench_shared_n${n}.wasm" -o "$OUT/bench_shared_n${n}_O4.wasm" 2>/dev/null

  # Specialize bench (uses original import names — bench_harness imports "lib" "malloc")
  # Merge bench harness + specialize library in one step
  ARGS="$OUT/bench_harness.wasm=inst0"
  for i in $(seq 1 $((n-1))); do ARGS="$ARGS $OUT/c${n}_${i}.wasm=inst${i}"; done
  ARGS="$ARGS $WASI_LIB=lib"
  $MERGE $ARGS --specialize --lib lib -o "$OUT/bench_spec_n${n}_raw.wasm" 2>/dev/null || true
  wasm-opt -O4 --enable-multimemory --enable-bulk-memory "$OUT/bench_spec_n${n}_raw.wasm" -o "$OUT/bench_spec_n${n}.wasm" 2>/dev/null || true

  echo "  N=$n: dispatch=$(wc -c < "$OUT/bench_shared_n${n}_O4.wasm" 2>/dev/null || echo 0) spec=$(wc -c < "$OUT/bench_spec_n${n}.wasm" 2>/dev/null || echo 0)"
done

echo ""
echo "=== Step 4: Run benchmarks ==="

# Run a bench module, return time in microseconds
run_wasmtime() {
  wasmtime run --wasm multi-memory "$1" 2>/dev/null | grep -o '[0-9]*'
}

run_v8() {
  node --input-type=module -e "
import {readFileSync} from 'fs';
const f = '$1';
const mod = new WebAssembly.Module(readFileSync(f));
const imports = {};
const mem_ref = [null];
for (const imp of WebAssembly.Module.imports(mod)) {
  if (!imports[imp.module]) imports[imp.module] = {};
  if (imp.kind === 'function') {
    if (imp.name === 'clock_time_get') {
      imports[imp.module][imp.name] = (id, prec, out) => {
        const ns = BigInt(Math.round(performance.now() * 1e6));
        new DataView(mem_ref[0].buffer).setBigInt64(out, ns, true);
        return 0;
      };
    } else if (imp.name === 'fd_write') {
      imports[imp.module][imp.name] = (fd, iovs, iovs_len, nwritten) => {
        const view = new DataView(mem_ref[0].buffer);
        let total = 0;
        for (let i = 0; i < iovs_len; i++) {
          const ptr = view.getUint32(iovs + i * 8, true);
          const len = view.getUint32(iovs + i * 8 + 4, true);
          process.stdout.write(Buffer.from(new Uint8Array(mem_ref[0].buffer, ptr, len)));
          total += len;
        }
        view.setUint32(nwritten, total, true);
        return 0;
      };
    } else {
      imports[imp.module][imp.name] = () => 0;
    }
  }
  else if (imp.kind === 'memory') { const m = new WebAssembly.Memory({initial:16}); imports[imp.module][imp.name] = m; mem_ref[0] = m; }
  else if (imp.kind === 'global') imports[imp.module][imp.name] = new WebAssembly.Global({value:'i32',mutable:true}, 0);
  else if (imp.kind === 'table') imports[imp.module][imp.name] = new WebAssembly.Table({initial:1,element:'anyfunc'});
}
const inst = new WebAssembly.Instance(mod, imports);
if (inst.exports.memory) mem_ref[0] = inst.exports.memory;
inst.exports._start();
" 2>&1 | grep -o '[0-9]*'
}

# Collect results as JSON
echo '{' > "$OUT/bench.json"
echo '  "config": {' >> "$OUT/bench.json"
echo '    "warmup_calls": 5000,' >> "$OUT/bench.json"
echo '    "measured_calls": 50000,' >> "$OUT/bench.json"
echo '    "workload": "malloc(8)+free per call",' >> "$OUT/bench.json"
echo '    "wasi_lib": "wasi-sdk 32, clang --target=wasm32-wasip1 -O2",' >> "$OUT/bench.json"
echo '    "runtimes": ["V8 (Node.js '$(node --version)')", "Wasmtime '$(wasmtime --version | head -1)'"],' >> "$OUT/bench.json"
echo '    "runs_per_measurement": 7' >> "$OUT/bench.json"
echo '  },' >> "$OUT/bench.json"

# Sizes
echo '  "sizes": {' >> "$OUT/bench.json"
BASELINE_SIZE=$(wc -c < "$OUT/baseline.wasm")
first=true
for n in "${Ns[@]}"; do
  SHARED_O4=$(wc -c < "$OUT/shared_n${n}_O4.wasm" 2>/dev/null || echo 0)
  SPEC_DCE=$(wc -c < "$OUT/spec_n${n}_dce.wasm" 2>/dev/null || echo 0)
  $first || echo ',' >> "$OUT/bench.json"
  first=false
  echo -n "    \"$n\": {\"baseline\": $((BASELINE_SIZE * n)), \"shared_O4\": $SHARED_O4, \"specialize_dce\": $SPEC_DCE}" >> "$OUT/bench.json"
done
echo '' >> "$OUT/bench.json"
echo '  },' >> "$OUT/bench.json"

# Benchmarks
echo '  "benchmarks": {' >> "$OUT/bench.json"

echo "  Baseline..."
echo -n '    "baseline": {"v8": [' >> "$OUT/bench.json"
for r in 1 2 3 4 5 6 7; do
  [ $r -gt 1 ] && echo -n ',' >> "$OUT/bench.json"
  echo -n "$(run_v8 "$OUT/bench_baseline.wasm")" >> "$OUT/bench.json"
done
echo -n '], "wasmtime": [' >> "$OUT/bench.json"
for r in 1 2 3 4 5 6 7; do
  [ $r -gt 1 ] && echo -n ',' >> "$OUT/bench.json"
  echo -n "$(run_wasmtime "$OUT/bench_baseline.wasm")" >> "$OUT/bench.json"
done
echo ']},' >> "$OUT/bench.json"

for n in "${Ns[@]}"; do
  echo "  N=$n..."
  for variant in "O4:bench_shared_n${n}_O4" "spec:bench_spec_n${n}"; do
    IFS=: read vlabel vfile <<< "$variant"
    BENCH_FILE="$OUT/${vfile}.wasm"
    if [ ! -f "$BENCH_FILE" ]; then
      echo -n "    \"n${n}_${vlabel}\": {\"v8\": [0,0,0], \"wasmtime\": [0,0,0]" >> "$OUT/bench.json"
    else
      echo -n "    \"n${n}_${vlabel}\": {\"v8\": [" >> "$OUT/bench.json"
      for r in 1 2 3 4 5 6 7; do
        [ $r -gt 1 ] && echo -n ',' >> "$OUT/bench.json"
        echo -n "$(run_v8 "$BENCH_FILE")" >> "$OUT/bench.json"
      done
      echo -n '], "wasmtime": [' >> "$OUT/bench.json"
      for r in 1 2 3 4 5 6 7; do
        [ $r -gt 1 ] && echo -n ',' >> "$OUT/bench.json"
        V=$(run_wasmtime "$BENCH_FILE" || echo 0)
        echo -n "${V:-0}" >> "$OUT/bench.json"
      done
    fi
    if [ "$n" = "100" ] && [ "$vlabel" = "spec" ]; then
      echo ']}'  >> "$OUT/bench.json"
    else
      echo ']},' >> "$OUT/bench.json"
    fi
  done
done

echo '  }' >> "$OUT/bench.json"
echo '}' >> "$OUT/bench.json"

echo ""
echo "=== Done ==="
echo "Results: $OUT/bench.json"
echo "Tables:  node bench-table.mjs"
