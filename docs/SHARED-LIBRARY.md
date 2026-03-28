## Prototype: Shared Library Linking for Multiply-Instantiated Wasm Modules

**Repo**: [https://github.com/ttraenkler/webassembly-intra-module-sandbox](https://github.com/ttraenkler/webassembly-intra-module-sandbox)

### Problem

When lowering components into flat core Wasm, shared libraries like wasi-libc may be instantiated N times. Today the options are: duplicate every function body N times (O(N) binary size), or keep the library as a separate module instantiated N times (each instance gets its own memory and globals, but prevents cross-module inlining in V8 which compiles at the module level). Core Wasm has no portable linking model and no way for a single function body to reference different memory indices for different callers — the memory index is a static immediate.

### Solution

**`wasm-merge`** merges a library with N consumer modules in one step. Two modes:

- **`--specialize` (inlined)**: analyzes the library's call graph, traces per-consumer reachability, and creates specialized copies of state-touching functions with the correct memory/global indices baked in. Pure functions stay shared. After `wasm-opt -O4`, shell wrappers are inlined — **faster than baseline**.
- **`--dispatch` (shared)**: uses `br_table` dispatch wrappers instead of duplication — **smallest binary** but runtime overhead per memory access.

Consumers import `malloc(size)` with the original signature — no modifications needed.

Tested with **real wasi-libc** (62KB, 199 functions, dlmalloc) — malloc, calloc, realloc, free, strlen all verified correct across 100 isolated instances.

### Results

Benchmarked with real wasi-libc, N=2 to N=100 consumers, on V8 and Cranelift. Inlined mode is **faster than baseline** on both runtimes after `wasm-opt -O4`. Shared mode trades performance for up to **98% binary size reduction**.

Full results: [BENCHMARK.md](BENCHMARK.md) — reproducible via `scripts/bench.sh && node scripts/bench-table.mjs > docs/BENCHMARK.md`

**Note**: separate instantiation (each consumer gets its own library instance via the runtime) is faster — zero dispatch overhead, just a cross-module call trampoline. This approach trades that for **single-module deployment** — useful when shipping one flat `.wasm` file to runtimes without native Component Model support.

### Connection to `i32.load_dynamic`

The `br_table` dispatch in shared mode is a polyfill for a missing instruction: `i32.load` with a runtime memory index (like `call` → `call_indirect` for functions). A `load_dynamic` instruction would give shared mode's size benefits with inlined mode's performance — see [DYNAMIC_MEMORY_INDEX.md](DYNAMIC_MEMORY_INDEX.md).

### Tools

- **wasm-merge** (~1400 lines Rust): multi-memory module merger with integrated inlined and shared modes. `--specialize --lib <label>` creates per-consumer specialized copies; `--dispatch --lib <label>` uses `br_table` wrappers for smallest binary. Consumers import standard function names (`malloc`, `free`) with no modifications. See [WASM-MERGE.md](WASM-MERGE.md).
