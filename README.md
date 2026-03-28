# WebAssembly shared-nothing linking at zero-cost possible?

While shared-nothing linking usually comes at the cost of function calls not being inlined and copying parameters today, shared-everything linking removes the sandbox isolation between modules.

Merging multiple Wasm modules into a single core module today means **shared-everything** — all functions share one memory, one global space, no isolation. This prototype demonstrates that modules can be merged while **maintaining shared-nothing isolation** between them, with **zero runtime cost** after optimization in many cases — no spec changes required.

## Cross-module function calls

Multi-memory lets each module keep its own linear memory after merge. Module A owns memory; Module B accesses it only through exported functions — never via a raw pointer. After merging and optimizing, the call is **completely eliminated**.

- **Primitives** (i32, i64, f32, f64): values pass on the Wasm stack — no memory copy. After inlining, `inc_and_get()` becomes a direct `global.set` + `global.get`.
- **Structs** via field accessors: `get_x()`, `set_y()` inline to direct `i32.load`/`i32.store` on A's memory at fixed offsets.
- **GC types** (Wasm GC proposal): `structref`/`arrayref` pass by reference on the stack — the runtime enforces type safety and field access, no linear memory or accessor functions needed. A natural complement for languages that target GC types instead of linear memory.
- **Memory regions**: bounds-checked byte accessors, opaque handles via funcref table.
- **Ownership**: read-only borrow, mutable borrow (call-scoped), move/transfer with use-after-move trap.

See [ACCESSOR.md](docs/ACCESSOR.md) for full WAT source, optimized disassembly, and benchmarks.

## Shared library linking

N consumer modules can share a single library (e.g. wasi-libc) while maintaining independent per-instance state. Two modes:

- **Inlined** (`--specialize`): per-consumer specialized copies, static memory indices. After `wasm-opt -O4` — **faster than baseline**.
- **Shared** (`--dispatch`): `br_table` dispatch, one copy of function bodies — **smallest binary** (-98% at N=100).

```bash
wasm-merge a.wasm=inst0 b.wasm=inst1 lib.wasm=lib --specialize --lib lib -o merged.wasm
wasm-merge a.wasm=inst0 b.wasm=inst1 lib.wasm=lib --dispatch --lib lib -o merged.wasm
```

See [SHARED-LIBRARY.md](docs/SHARED-LIBRARY.md) for the problem statement and [BENCHMARK.md](docs/BENCHMARK.md) for results.

## Documentation

- [WASM-MERGE.md](docs/WASM-MERGE.md) — tool usage, modes, how specialization works
- [SHARED-LIBRARY.md](docs/SHARED-LIBRARY.md) — multiply-instantiated shared libraries
- [BENCHMARK.md](docs/BENCHMARK.md) — binary size and runtime performance (V8, Cranelift)
- [ACCESSOR.md](docs/ACCESSOR.md) — shared memory accessor security and ownership patterns
- [DYNAMIC_MEMORY_INDEX.md](docs/DYNAMIC_MEMORY_INDEX.md) — motivation for `i32.load_dynamic` spec proposal

## Files

| Directory           | Contents                                                                                                                   |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `input/accessor/`   | WAT examples: security patterns, ownership patterns, component model                                                       |
| `input/shared-lib/` | Shared library demo (`libc_subset.wat`, `wasi-libc.wasm`)                                                                  |
| `scripts/`          | `run-all.sh`, `demo-sandbox-merge.sh`, `demo-shared-memory-accessor.sh`, `benchmark-shared-library.sh`, `bench-format.mjs` |
| `wasm-merge/`       | Multi-memory merger (Rust)                                                                                                 |
| `docs/`             | Documentation                                                                                                              |
| `output/`           | Generated artifacts (gitignored)                                                                                           |

## Prerequisites

- **Rust** — [rustup.rs](https://rustup.rs)
- **wasm-tools** — `cargo install wasm-tools`
- **Binaryen** (optional) — `wasm-opt` for inlining and optimization

```bash
cd wasm-merge && cargo build --release
```

## Run

```bash
scripts/run-all.sh                         # run everything

# Or individually:
scripts/demo-sandbox-merge.sh              # basic merge demo
scripts/demo-shared-memory-accessor.sh     # generates docs/ACCESSOR.md
scripts/benchmark-shared-library.sh        # full benchmarks → output/bench.json
node scripts/bench-format.mjs > docs/BENCHMARK.md
```
