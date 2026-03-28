// bench-format.mjs — generate markdown tables from output/bench.json
// Usage: node scripts/bench-format.mjs > docs/BENCHMARK.md

import { readFileSync } from 'fs';

const data = JSON.parse(readFileSync('output/bench.json', 'utf-8'));
const cfg = data.config;

// Helper: median of array, drop first element (cold JIT), filter zeros (failed runs)
const median = arr => {
  const warm = arr.slice(1).filter(x => x > 0);
  if (warm.length === 0) return null;
  warm.sort((a, b) => a - b);
  return warm[Math.floor(warm.length / 2)];
};

console.log(`## Benchmark Results`);
console.log('');
console.log(`- **Workload**: ${cfg.measured_calls.toLocaleString()} ${cfg.workload} (${cfg.warmup_calls.toLocaleString()} warmup)`);
console.log(`- **Library**: ${cfg.wasi_lib}`);
console.log(`- **Runtimes**: ${cfg.runtimes.join(', ')}`);
console.log(`- **Runs**: ${cfg.runs_per_measurement} per measurement (first dropped as cold, median of rest)`);
console.log(`- **Reproducible**: \`scripts/benchmark-shared-library.sh && node scripts/bench-format.mjs > docs/BENCHMARK.md\``);
console.log('');

// Size table
console.log('### Binary size (bytes)');
console.log('');
console.log('| N | Baseline | Shared -O4 | Δ | Inlined (DCE) | Δ |');
console.log('|---|---|---|---|---|---|');
for (const [n, s] of Object.entries(data.sizes)) {
  const d4 = ((s.shared_O4 / s.baseline - 1) * 100).toFixed(0);
  const spec = s.specialize_dce || 0;
  const ds = spec ? ((spec / s.baseline - 1) * 100).toFixed(0) + '%' : 'n/a';
  console.log(`| ${n} | ${s.baseline.toLocaleString()} | ${s.shared_O4.toLocaleString()} | ${d4}% | ${spec ? spec.toLocaleString() : 'n/a'} | ${ds} |`);
}
console.log('');
console.log('- **N**: number of consumer modules sharing the library');
console.log('- **Baseline**: N separate copies of (consumer + library), each independently merged');
console.log('- **Shared -O4**: shared library with `br_table` dispatch wrappers, after `wasm-opt -O4`');
console.log('- **Inlined (DCE)**: per-instance specialized copies of state-touching functions, dead code eliminated — zero dispatch overhead, pure functions shared');
console.log('- **Δ**: size change vs Baseline (negative = smaller = better)');

// Performance table — merged rows
const b = data.benchmarks;
const base_v8 = median(b.baseline.v8);
const base_wt = median(b.baseline.wasmtime);

console.log('');
console.log('### Runtime performance — V8 vs Cranelift (μs, pure Wasm loop)');
console.log('');
console.log('| N | V8 Shared (μs) | V8 Inlined (μs) | Cranelift Shared (μs) | Cranelift Inlined (μs) |');
console.log('|---|---|---|---|---|');
console.log(`| — (baseline) | ${base_v8} | ${base_v8} | ${base_wt} | ${base_wt} |`);

for (const n of Object.keys(data.sizes)) {
  const dk = `n${n}_O4`;
  const sk = `n${n}_spec`;
  const dv8  = b[dk] ? median(b[dk].v8)       : null;
  const sv8  = b[sk] ? median(b[sk].v8)       : null;
  const dwt  = b[dk] ? median(b[dk].wasmtime) : null;
  const swt  = b[sk] ? median(b[sk].wasmtime) : null;
  const cell = (v, base) => {
    if (v == null) return 'n/a';
    if (!base) return `${v}`;
    const raw = (v / base - 1) * 100;
    const pct = raw.toFixed(0);
    return `${v} (${raw >= 0 ? '+' : ''}${pct}%)`;
  };
  console.log(`| ${n} | ${cell(dv8, base_v8)} | ${cell(sv8, base_v8)} | ${cell(dwt, base_wt)} | ${cell(swt, base_wt)} |`);
}
console.log('');
console.log('- **Baseline**: single consumer merged with unmodified wasi-libc — direct static memory and global access, no dispatch overhead');
console.log('- **Shared**: shared library with `br_table` dispatch, after `wasm-opt -O4` — one copy of function bodies, dispatch on every memory access');
console.log('- **Inlined**: per-instance specialized copies with memory indices baked in — zero dispatch overhead, larger binary');
console.log('- **Δ**: overhead vs baseline (lower = better)');
console.log('- **V8 (TurboFan)**: JIT-compiled. Handles `br_table` dispatch with ~2-3× overhead via branch prediction.');
console.log('- **Cranelift**: AOT-compiled by Wasmtime. Shows ~25× overhead for dispatch — less optimization of `br_table` hot paths.');
console.log('- **n/a**: Wasmtime may fail at high N due to too many imported memories.');
