## Benchmark Results

- **Workload**: 50,000 malloc(8)+free per call (5,000 warmup)
- **Library**: wasi-sdk 32, clang --target=wasm32-wasip1 -O2
- **Runtimes**: V8 (Node.js v25.8.1), Wasmtime wasmtime 43.0.0
- **Runs**: 7 per measurement (first dropped as cold, median of rest)
- **Reproducible**: `scripts/benchmark-shared-library.sh && node scripts/bench-format.mjs > docs/BENCHMARK.md`

### Binary size (bytes)


| N   | Baseline  | Shared -O4 | Δ    | Inlined (DCE) | Δ    |
| --- | --------- | ---------- | ---- | ------------- | ---- |
| 2   | 32,242    | 42,713     | 32%  | 33,086        | 3%   |
| 5   | 80,605    | 18,373     | -77% | 56,827        | -29% |
| 10  | 161,210   | 19,208     | -88% | 96,422        | -40% |
| 20  | 322,420   | 20,945     | -94% | 175,684       | -46% |
| 50  | 806,050   | 26,199     | -97% | 413,525       | -49% |
| 100 | 1,612,100 | 34,986     | -98% | 809,926       | -50% |


- **N**: number of consumer modules sharing the library
- **Baseline**: N separate copies of (consumer + library), each independently merged
- **Shared -O4**: shared library with `br_table` dispatch wrappers, after `wasm-opt -O4`
- **Inlined (DCE)**: per-instance specialized copies of state-touching functions, dead code eliminated — zero dispatch overhead, pure functions shared
- **Δ**: size change vs Baseline (negative = smaller = better)

### Runtime performance — V8 vs Cranelift (μs, pure Wasm loop)


| N            | V8 Shared (μs) | V8 Inlined (μs) | Cranelift Shared (μs) | Cranelift Inlined (μs) |
| ------------ | -------------- | --------------- | --------------------- | ---------------------- |
| — (baseline) | 961            | 961             | 405                   | 405                    |
| 2            | 1817 (+89%)    | 699 (-27%)      | 12740 (+3046%)        | 400 (-1%)              |
| 5            | 2530 (+163%)   | 764 (-20%)      | 13065 (+3126%)        | 406 (+0%)              |
| 10           | 2634 (+174%)   | 741 (-23%)      | 14552 (+3493%)        | 385 (-5%)              |
| 20           | 5284 (+450%)   | 761 (-21%)      | 16033 (+3859%)        | 355 (-12%)             |
| 50           | 2824 (+194%)   | 708 (-26%)      | 13111 (+3137%)        | 365 (-10%)             |
| 100          | 3501 (+264%)   | 691 (-28%)      | 13349 (+3196%)        | 377 (-7%)              |


- **Baseline**: single consumer merged with unmodified wasi-libc — direct static memory and global access, no dispatch overhead
- **Shared**: shared library with `br_table` dispatch, after `wasm-opt -O4` — one copy of function bodies, dispatch on every memory access
- **Inlined**: per-instance specialized copies with memory indices baked in — zero dispatch overhead, larger binary
- **Δ**: overhead vs baseline (lower = better)
- **V8 (TurboFan)**: JIT-compiled. Handles `br_table` dispatch with ~2-3× overhead via branch prediction.
- **Cranelift**: AOT-compiled by Wasmtime. Shows ~25× overhead for dispatch — less optimization of `br_table` hot paths.
- **n/a**: Wasmtime may fail at high N due to too many imported memories.

