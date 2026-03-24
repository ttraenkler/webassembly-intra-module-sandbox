# WebAssembly Intra-Module Sandboxing — Zero-Cost Abstraction Demo

This prototype demonstrates that **intra-module sandboxing** in WebAssembly can be achieved with **zero runtime cost** using existing tooling — no spec changes required.

## The Idea

Two Wasm modules communicate through an accessor function interface:

- **Module A** owns a linear memory containing data (`"hello"`)
- **Module B** can read A's data _only_ through A's exported `string_byte(i)` function
- B never receives a raw pointer into A's memory and cannot forge one

After merging with multi-memory and optimizing with inlining:

1. Both memories are preserved as separate address spaces (memory 0, memory 1)
2. The accessor function call is **completely eliminated**
3. `read_first()` compiles to a direct `i32.load8_u 1` — identical to shared-everything

## Files

| File            | Description                                             |
| --------------- | ------------------------------------------------------- |
| `a.wat`         | Module A — owns memory, exports `string_byte` accessor  |
| `b.wat`         | Module B — imports accessor, has its own memory         |
| `run.sh`        | Full pipeline: compile → merge → verify                 |
| `component.wat` | Same example using Component Model nested-module syntax |
| `wasm-merge/`   | Rust tool: shared-nothing multi-memory module merger    |

## Prerequisites

- **Rust** — [rustup.rs](https://rustup.rs)

```bash
# Install wasm-tools (WAT ↔ Wasm, component parsing)
cargo install wasm-tools

# Build wasm-merge
cd wasm-merge && cargo build --release
```

Optionally install [Binaryen](https://github.com/WebAssembly/binaryen/releases) for `wasm-opt --inlining` (eliminates cross-module call overhead after merge).

## Run

```bash
./run.sh
```

## wasm-merge

A shared-nothing multi-memory module merger, inspired by and ported from [Binaryen](https://github.com/WebAssembly/binaryen)'s `wasm-merge` by Alon Zakai ([@kripken](https://github.com/kripken)).

Unlike Binaryen's `wasm-merge` (C++, 8.7 MB), this is written in Rust using the `wasmparser` and `wasm-encoder` crates from [bytecodealliance/wasm-tools](https://github.com/bytecodealliance/wasm-tools), and compiles to a 607 KB WASI module that runs in any Wasm runtime.

### Usage

```bash
# Merge standalone modules
wasm-merge b.wasm=b a.wasm=a -o merged.wasm

# Merge from a binary component (auto-detected with single file)
wasm-merge component.wasm -o merged.wasm

# Build as WASI module
cd wasm-merge && cargo build --target wasm32-wasip1 --release
```

Each module keeps its own linear memory (multi-memory). Cross-module imports
are resolved by matching labels to export namespaces.

### How it works

1. Parses each core module with `wasmparser`
2. Builds an index remapping table (functions, memories, globals, tables, types)
3. Uses `wasm-encoder`'s `Reencode` trait to transcribe all sections with remapped indices
4. Emits a single core module where each original module's memory is a distinct memory index

For components, it additionally extracts nested `(core module ...)` definitions
and reads `(core instance ...)` declarations to determine import wiring.

## Key Results

**After merge** (two memories, accessor call preserved):

```wat
(func (;1;) (type 1) (param i32) (result i32)
  local.get 0
  i32.load8_u 1)        ;; ← loads from memory 1 (A's memory)

(func (;0;) (type 0) (result i32)
  i32.const 0
  call 1)               ;; ← still calls string_byte
```

**After inlining** (call eliminated, direct memory access):

```wat
(func (;0;) (type 0) (result i32)
  i32.const 0
  i32.load8_u 1)        ;; ← direct load from A's memory, no call!
```

## Why This Matters

- **Isolation at the interface level**: modules cannot access each other's memory
- **Zero-cost abstraction**: the optimizer erases the boundary entirely
- **No spec changes needed**: works today with multi-memory + existing tooling
- **Component Model alignment**: the same pattern maps directly to nested core modules
