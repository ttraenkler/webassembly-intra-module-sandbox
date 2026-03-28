// bench.mjs — run all benchmarks, write results to output/bench.json
// Usage: node bench.mjs

import { readFileSync, writeFileSync, mkdirSync } from 'fs';

const load = f => {
  const mod = new WebAssembly.Module(readFileSync(f));
  const imports = {};
  for (const imp of WebAssembly.Module.imports(mod)) {
    if (!imports[imp.module]) imports[imp.module] = {};
    if (imp.kind === 'function') imports[imp.module][imp.name] = () => 0;
    else if (imp.kind === 'memory') imports[imp.module][imp.name] = new WebAssembly.Memory({ initial: 16 });
    else if (imp.kind === 'global') imports[imp.module][imp.name] = new WebAssembly.Global({ value: 'i32', mutable: true }, 0);
    else if (imp.kind === 'table') imports[imp.module][imp.name] = new WebAssembly.Table({ initial: 1, element: 'anyfunc' });
  }
  return new WebAssembly.Instance(mod, imports);
};

const N_CALLS = 5000;
const REPS = 50;

function bench(file, sfx, workload) {
  const inst = load(file);
  const e = inst.exports;
  // warmup
  for (let i = 0; i < 2000; i++) workload(e, sfx);
  const times = [];
  for (let r = 0; r < REPS; r++) {
    const t0 = performance.now();
    for (let i = 0; i < N_CALLS; i++) workload(e, sfx);
    times.push(performance.now() - t0);
  }
  times.sort((a, b) => a - b);
  return {
    median: times[Math.floor(times.length / 2)],
    min: times[0],
    p25: times[Math.floor(times.length * 0.25)],
    p75: times[Math.floor(times.length * 0.75)],
  };
}

const workloads = {
  'malloc+free': (e, s) => { e['free' + s](e['malloc' + s](64)); },
  'calloc+free': (e, s) => { e['free' + s](e['calloc' + s](10, 4)); },
  'malloc+realloc+free': (e, s) => { const p = e['malloc' + s](16); const q = e['realloc' + s](p, 128); e['free' + s](q); },
  'strlen': (e, s) => { e['strlen' + s](0); },
};

const Ns = [2, 5, 10, 20, 50, 100];

// Pre-warm JIT for all module shapes
console.error('Pre-warming JIT...');
for (const fn of Object.values(workloads)) {
  const w = load('/tmp/base_big_merged.wasm');
  for (let i = 0; i < 500; i++) fn(w.exports, '');
}
for (const n of Ns) {
  const sfx = '__inst' + (n - 1);
  try {
    const w = load('/tmp/param' + n + '_perf.wasm');
    for (let i = 0; i < 500; i++) w.exports['free' + sfx](w.exports['malloc' + sfx](8));
  } catch (e) {}
}

const results = {
  config: { calls: N_CALLS, reps: REPS, runtime: 'V8 (Node.js ' + process.version + ')' },
  per_function: {},
  per_n: {},
};

// Per-function table (N=50)
console.error('Benchmarking per-function (N=50)...');
for (const [name, fn] of Object.entries(workloads)) {
  const baseline = bench('/tmp/base_big_merged.wasm', '', fn);
  const shared = bench('/tmp/param50_perf.wasm', '__inst49', fn);
  results.per_function[name] = { baseline, shared };
  console.error(`  ${name}: baseline=${baseline.median.toFixed(3)}ms shared=${shared.median.toFixed(3)}ms`);
}

// Per-N table (malloc+free)
console.error('Benchmarking per-N (malloc+free)...');
for (const n of Ns) {
  const baseline = bench('/tmp/base_big_merged.wasm', '', workloads['malloc+free']);
  const shared = bench('/tmp/param' + n + '_perf.wasm', '__inst' + (n - 1), workloads['malloc+free']);
  results.per_n[n] = { baseline, shared };
  console.error(`  N=${n}: baseline=${baseline.median.toFixed(3)}ms shared=${shared.median.toFixed(3)}ms`);
}

// Size data
console.error('Collecting sizes...');
results.sizes = {};
for (const n of Ns) {
  try {
    const baseline_size = readFileSync('/tmp/real_full_' + n + '/base_opt.wasm').length * n;
    const shared_size = readFileSync('/tmp/param' + n + '_perf.wasm').length;
    results.sizes[n] = { baseline: baseline_size, shared: shared_size };
  } catch (e) {}
}

mkdirSync('output', { recursive: true });
writeFileSync('output/bench.json', JSON.stringify(results, null, 2));
console.error('Wrote output/bench.json');
