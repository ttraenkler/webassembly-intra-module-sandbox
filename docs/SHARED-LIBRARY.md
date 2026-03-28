## Prototype: Shared Library Linking for Multiple Instantiated Wasm Modules

**Repo**: [https://github.com/ttraenkler/webassembly-intra-module-sandbox](https://github.com/ttraenkler/webassembly-intra-module-sandbox)

### Problem

When lowering components into flat core Wasm, shared libraries like wasi-libc may be instantiated N times — once per consumer that imports it. Each instance needs its own memory and globals (heap, stack pointer). After merging into a single multi-memory module, each consumer's memory becomes a separate memory index (0, 1, 2, ...).

The core constraint: **every `i32.load`/`i32.store` in Wasm takes the memory index as a static immediate** — baked into the bytecode at compile time. A single `malloc` function body can only say `i32.load memory=0`; it cannot say "load from whichever memory my caller uses." There is no `i32.load memory=$variable`.

This means a shared `malloc` that serves N consumers with N different memories cannot exist as a single function body in core Wasm today. The options are:

1. **Duplicate** every function body N times, each hardcoded to a different memory — O(N) binary size
2. **Keep separate modules** — the runtime instantiates the library N times (correct, but prevents cross-module inlining in V8 which compiles at the module level)
3. **Dispatch at runtime** — wrap every memory access in a `br_table` that selects the correct memory index based on a parameter: one function body, but overhead per memory access

### Why `br_table`?

Since Wasm has no instruction for "load from memory index on the stack", the dispatch wrapper emulates it:

```wat
;; One shared function body for i32.load across N memories
(func $dispatch_i32_load (param $addr i32) (param $instance i32) (result i32)
  (block $default (block $b1 (block $b0
    (br_table $b0 $b1 $default (local.get $instance)))
    (return (i32.load memory=0 (local.get $addr))))  ;; instance 0
    (return (i32.load memory=1 (local.get $addr))))  ;; instance 1
  (unreachable))
```

Every `i32.load` in the shared library is replaced with a call to this wrapper, passing the instance index. This is the smallest possible binary (one copy of each function), but adds a function call + branch per memory access.

The **inlined mode** avoids this by creating N copies of each state-touching function with the correct memory index baked in — no dispatch, but O(N) for those functions (pure functions stay shared).

### Solution

**`wasm-merge`** merges a library with N consumer modules in one step. Two modes:

- **`--specialize` (inlined)**: analyzes the library's call graph, traces per-consumer reachability, and creates specialized copies of state-touching functions with the correct memory/global indices baked in. Pure functions stay shared. After `wasm-opt -O4`, shell wrappers are inlined — **faster than baseline**.
- **`--dispatch` (shared)**: uses `br_table` dispatch wrappers instead of duplication — **smallest binary** but runtime overhead per memory access.

Consumers import `malloc(size)` with the original signature — no modifications needed.

Tested with **real wasi-libc** (62KB, 199 functions, dlmalloc) — malloc, calloc, realloc, free, strlen all verified correct across 100 isolated instances.

### Results

Benchmarked with real wasi-libc, N=2 to N=100 consumers, on V8 and Cranelift. Inlined mode is **faster than baseline** on both runtimes after `wasm-opt -O4`. Shared mode trades performance for up to **98% binary size reduction**.

Full results: [BENCHMARK.md](BENCHMARK.md) — reproducible via `scripts/benchmark-shared-library.sh && node scripts/bench-format.mjs > docs/BENCHMARK.md`

**Note**: separate instantiation (each consumer gets its own library instance via the runtime) is faster — zero dispatch overhead, just a cross-module call trampoline. This approach trades that for **single-module deployment** — useful when shipping one flat `.wasm` file to runtimes without native Component Model support.

### Connection to `i32.load_dynamic`

The `br_table` dispatch in shared mode is a polyfill for a missing instruction: `i32.load` with a runtime memory index (like `call` → `call_indirect` for functions). A `load_dynamic` instruction would give shared mode's size benefits with inlined mode's performance — see [DYNAMIC_MEMORY_INDEX.md](DYNAMIC_MEMORY_INDEX.md).

### Tools

- **wasm-merge** (~1400 lines Rust): multi-memory module merger with integrated inlined and shared modes. `--specialize --lib <label>` creates per-consumer specialized copies; `--dispatch --lib <label>` uses `br_table` wrappers for smallest binary. Consumers import standard function names (`malloc`, `free`) with no modifications. See [WASM-MERGE.md](WASM-MERGE.md).
