# WebAssembly Intra-Module Sandboxing — Zero-Cost Abstraction Demo

This prototype demonstrates that **intra-module sandboxing** in WebAssembly can
be achieved with **zero runtime cost** using existing tooling — no spec changes
required.

## The Idea

Two Wasm modules communicate through an accessor function interface:

- **Module A** owns a linear memory containing data (`"hello"`)
- **Module B** can read A's data *only* through A's exported `string_byte(i)` function
- B never receives a raw pointer into A's memory and cannot forge one

After merging with `wasm-merge` (multi-memory) and optimizing with `wasm-opt --inlining`:

1. Both memories are preserved as separate address spaces (memory 0, memory 1)
2. The accessor function call is **completely eliminated**
3. `read_first()` compiles to a direct `i32.load8_u 1` — identical to shared-everything

## Files

| File | Description |
|------|-------------|
| `a.wat` | Module A — owns memory, exports `string_byte` accessor |
| `b.wat` | Module B — imports accessor, has its own memory |
| `run.sh` | Full pipeline: compile → merge → verify → optimize → verify |
| `component.wat` | Stretch goal: Component Model nested-module version |

## Prerequisites

- **Binaryen** ≥ 116 (provides `wasm-merge` and `wasm-opt`)
- **WABT** (provides `wat2wasm` and `wasm2wat`)

```bash
# Ubuntu/Debian
apt install binaryen wabt
# Or via npm (for latest wasm-merge)
npm install -g binaryen
```

## Run

```bash
./run.sh
```

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
- **No spec changes needed**: works today with multi-memory + Binaryen
- **Component Model alignment**: the same pattern maps directly to nested core modules
