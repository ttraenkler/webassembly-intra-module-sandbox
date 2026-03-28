# WebAssembly Intra-Module Sandboxing — Zero-Cost Abstraction Demo

This prototype demonstrates that **intra-module sandboxing** in WebAssembly can be achieved with **zero runtime cost** using existing tooling — no spec changes required.

Two Wasm modules communicate through an accessor function interface. After merging with multi-memory and optimizing with inlining, the accessor call is **completely eliminated** — `read_first()` compiles to a direct `i32.load8_u 1`, identical to shared-everything.

## Shared Library Linking

N consumer modules can share a single library (e.g. wasi-libc) while maintaining independent per-instance state. Two modes:

- **Inlined** (`--specialize`): per-consumer specialized copies, static memory indices. After `wasm-opt -O4` — **faster than baseline**.
- **Shared** (`--dispatch`): `br_table` dispatch, one copy of function bodies — **smallest binary** (-98% at N=100).

```bash
wasm-merge a.wasm=inst0 b.wasm=inst1 lib.wasm=lib --specialize --lib lib -o merged.wasm
wasm-merge a.wasm=inst0 b.wasm=inst1 lib.wasm=lib --dispatch --lib lib -o merged.wasm
```

## Documentation

- [WASM-MERGE.md](docs/WASM-MERGE.md) — tool usage, modes, how specialization works
- [SHARED-LIBRARY.md](docs/SHARED-LIBRARY.md) — problem statement and solution for multiply-instantiated shared libraries
- [BENCHMARK.md](docs/BENCHMARK.md) — binary size and runtime performance (V8, Cranelift), N=2 to N=100
- [SECURITY.md](docs/SECURITY.md) — security comparison: insecure vs bounds-checked vs table indirection
- [DYNAMIC_MEMORY_INDEX.md](docs/DYNAMIC_MEMORY_INDEX.md) — motivation for `i32.load_dynamic` spec proposal

## Files

| Directory | Contents |
|-----------|----------|
| `input/` | Source `.wat` modules and `wasi-libc.wasm` |
| `scripts/` | `run.sh`, `run_security.sh`, `bench.sh`, `bench-table.mjs` |
| `wasm-merge/` | Multi-memory merger (Rust) |
| `docs/` | Documentation |
| `output/` | Generated artifacts (gitignored) |

## Prerequisites

- **Rust** — [rustup.rs](https://rustup.rs)
- **wasm-tools** — `cargo install wasm-tools`
- **Binaryen** (optional) — `wasm-opt` for inlining and optimization

```bash
cd wasm-merge && cargo build --release
```

## Run

```bash
scripts/run.sh                  # basic merge demo
scripts/run_security.sh         # generates docs/SECURITY.md
scripts/bench.sh                # full benchmarks → output/bench.json
node scripts/bench-table.mjs > docs/BENCHMARK.md
```
