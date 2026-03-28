# Motivation: Dynamic Memory Index Instructions for WebAssembly

## Summary

This document motivates a new WebAssembly instruction variant that accepts a
**runtime memory index** on the stack, rather than a static immediate. We
demonstrate that the absence of this feature forces toolchains to emit O(N)
`br_table` dispatch wrappers for multiply-instantiated shared libraries, and
that a single spec change would eliminate this overhead entirely.

## The Problem

The [multi-memory proposal](https://github.com/WebAssembly/multi-memory)
(shipped in Wasm 3.0) allows modules to declare multiple linear memories.
Every memory instruction takes a memory index — but this index is a **static
immediate** encoded in the binary:

```wat
i32.load memory=2    ;; static — baked into bytecode
```

This works when each function body is used by a single consumer with a known
memory layout. It breaks when a **shared library** is merged into a module
that serves multiple consumers, each with their own memory.

### Concrete scenario: multiply-instantiated wasi-libc

When lowering a WebAssembly component into a flat core module (the
[lower-component RFC](https://github.com/nicofuchs/component-model/blob/main/design/mvp/lower-component/Explainer.md)),
shared libraries like wasi-libc may be instantiated N times — once per
consumer module that imports it. Each instance needs its own memory (heap,
stack) and its own mutable globals (`__stack_pointer`, `__heap_end`).

Today, the only options are:

1. **Duplicate** every function body N times, each with different static
   memory indices — O(N) binary size
2. **Reject** components that instantiate a module more than once
3. **Preserve module boundaries** — correct but prevents cross-module
   inlining in runtimes like V8 that compile at the module level

## The Polyfill: br_table Dispatch Wrappers

We built a [prototype](https://github.com/ttraenkler/webassembly-intra-module-sandbox) that achieves shared
function bodies with per-instance state by emitting `br_table` dispatch
wrappers:

```wat
;; Dispatch wrapper: replaces every i32.load in the shared library
(func $__dispatch_i32_load (param $addr i32) (param $instance_idx i32) (result i32)
  (block $default
    (block $b1
      (block $b0
        (br_table $b0 $b1 $default (local.get $instance_idx)))
      (return (i32.load memory 0 (local.get $addr))))    ;; instance 0
    (return (i32.load memory 1 (local.get $addr))))      ;; instance 1
  (unreachable))
```

Every memory access in the shared library is replaced with a call to the
appropriate dispatch wrapper. The `$instance_idx` is threaded as a function
parameter (foldable by ahead-of-time optimizers like wasm-opt).

### What this costs

Measured with real wasi-libc (62KB, dlmalloc), 50K malloc+free calls, N=50
instances, pure Wasm timing loop:

| Approach | V8 (TurboFan) | Cranelift (Wasmtime) |
|---|---|---|
| Baseline (direct static indices) | — | — |
| Shared (`br_table` dispatch, `-O4`) | +176% | +2954% |
| Inlined (per-instance copies, `-O4`) | **-25%** | **-16%** |

The `br_table` dispatch adds **+176% overhead on V8** and **+2954% on
Cranelift** for a memory-heavy workload like dlmalloc. Cranelift's AOT
compilation doesn't optimize `br_table` hot paths as aggressively as V8's
TurboFan JIT.

The inlined mode — specialized copies per instance with the correct memory
index baked in, then optimized with `wasm-opt -O4` — is **faster than
baseline** (-25% on V8, -16% on Cranelift) because the optimizer inlines
the shell wrappers and the static memory indices enable better code
generation. However, inlined mode only achieves 49% size reduction at N=50
compared to 97% for shared mode.

### What this requires from the toolchain

The dispatch tool (`wasm-merge --dispatch`) must:

1. Analyze the library's call graph to find all state-touching functions
2. Create N copies of each mutable global (one per instance)
3. Generate N-branch `br_table` dispatch wrappers for every load/store
   width, every mutable global, and memory.size/memory.grow
4. Thread `$instance_idx` through every state-touching function's parameter
   list
5. Shift all local indices in rewritten functions
6. Generate per-consumer stubs that hardwire the instance index

The prototype is ~1400 lines of Rust with two merge modes (shared and
inlined). It handles all i32/i64 load/store widths,
memory.fill/copy/grow/size, and mutable globals on real wasi-libc.

## The Proposed Spec Change

Add variants of every memory instruction that take the memory index from the
stack instead of a static immediate:

```wat
;; Current (static memory index):
i32.load memory=2 offset=0 align=2    ;; memory index is an immediate

;; Proposed (dynamic memory index):
i32.load_dynamic offset=0 align=2     ;; pops memory index from stack
```

### Semantics

`i32.load_dynamic offset=N align=A` pops two values from the stack:

1. `$memory_idx : i32` — the memory to load from (validated at runtime
   against the module's memory count)
2. `$addr : i32` — the address within that memory

It is equivalent to a `br_table` dispatch over all declared memories, with
each branch performing the corresponding static `i32.load memory=K`. The
runtime may implement it more efficiently (e.g., a memory base pointer table
lookup).

The same pattern applies to all memory instructions:

```
i32.load_dynamic, i64.load_dynamic, f32.load_dynamic, f64.load_dynamic
i32.load8_s_dynamic, i32.load8_u_dynamic, i32.load16_s_dynamic, ...
i32.store_dynamic, i64.store_dynamic, f32.store_dynamic, f64.store_dynamic
i32.store8_dynamic, i32.store16_dynamic, ...
memory.size_dynamic, memory.grow_dynamic
memory.fill_dynamic, memory.copy_dynamic
```

### Validation

- The `$memory_idx` must be an `i32` on the stack
- At runtime, if `$memory_idx >= memory_count`, trap
- Static analysis cannot determine which memory is accessed — same as
  `call_indirect` where the callee is determined at runtime

### What this eliminates

With `i32.load_dynamic`, the dispatch wrapper above becomes:

```wat
;; No wrapper needed. The shared library function directly uses:
(i32.load_dynamic offset=0 align=2
  (local.get $instance_idx)    ;; memory index from parameter
  (local.get $addr))           ;; address
```

The entire dispatch infrastructure — br_table wrappers, N-branch code
generation, call overhead per memory access — is replaced by a single
instruction that the runtime can implement as a pointer table lookup.

### Performance expectation

A `load_dynamic` with a constant `$memory_idx` should optimize to exactly
the same code as a static `i32.load memory=K`:

1. JIT sees `$memory_idx` is a known constant (inlined from caller)
2. Resolves to a specific memory base pointer
3. Emits a direct memory access — identical to static indexing

For a non-constant `$memory_idx`, the runtime emits a table lookup:

```
base_ptr = instance.memory_bases[$memory_idx]
result = *(base_ptr + $addr + offset)
```

This is one extra indirection — significantly cheaper than a `br_table`
with N branches and a function call.

## Relationship to Existing Work

### Multi-memory (Wasm 3.0)

Multi-memory added static memory indices. Dynamic memory indices are the
natural next step — the same relationship as `call` (static function index)
to `call_indirect` (dynamic function index via table).

### design#1439 — Fine-grained control of memory

Deepti Gandluri's 2022 exploration included "first-class memories" with
`ref.mem` as Option 2. The community deferred it. Dynamic memory indices
are a much smaller spec change that addresses the most pressing use case
(multiply-instantiated shared libraries) without requiring a full
reference-type extension for memories.

### component-model#626 and the lower-component RFC

The multiply-instantiated modules open question in the lower-component RFC
is the direct motivation for this work. The `br_table` polyfill
demonstrated here works today with no spec changes, but a dynamic memory
index instruction would:

- Eliminate the toolchain complexity (no dispatch wrappers, no call graph
  rewriting)
- Eliminate the cold-path overhead (no br_table dispatch on every access)
- Enable runtimes to implement the lookup as a simple pointer table —
  likely zero overhead for constant indices, near-zero for dynamic

### call_indirect precedent

WebAssembly already has the static-vs-dynamic pattern for function calls:

| | Static | Dynamic |
|---|---|---|
| Functions | `call $func_idx` | `call_indirect (type $sig) $table_idx` |
| Memories | `i32.load memory=$idx` | **proposed: `i32.load_dynamic`** |
| Globals | `global.get $idx` | *(not proposed — fewer globals, br_table sufficient)* |

The `call_indirect` precedent shows the community has accepted runtime
dispatch for functions. The same reasoning applies to memories: when the
index is statically known, the optimizer folds it; when it's dynamic, the
runtime pays a small lookup cost.

## Implementation Considerations

### For runtimes

Each Wasm instance already maintains a table of memory base pointers
(needed for multi-memory). A `load_dynamic` instruction just indexes into
this existing table at runtime instead of at compile time. In optimizing
tiers (TurboFan, Cranelift), constant propagation handles the common case
where the memory index is known.

### For validators

The `$memory_idx` is runtime-validated, similar to `call_indirect`'s table
index. The validator only needs to check that the instruction's static type
signature is correct (i32 on stack for the memory index).

### For toolchains

Toolchains generating code for multiply-instantiated shared libraries
would emit `load_dynamic`/`store_dynamic` with a `$instance_idx` parameter
instead of building dispatch wrapper infrastructure. The code generation
is straightforward — replace the static memory immediate with a stack
operand.

## Conclusion

The `br_table` dispatch polyfill demonstrated in this repository proves
that multiply-instantiated shared libraries with per-instance state are
achievable today with existing Wasm features. However:

- **Shared mode** (shared function bodies, `br_table` dispatch): **98%
  binary size reduction** at N=100, but +176% overhead on V8 and +2954%
  on Cranelift for memory-heavy workloads. The `br_table` dispatch on
  every memory access is the bottleneck.
- **Inlined mode** (per-instance copies, `-O4`): **faster than baseline**
  (-25% V8, -16% Cranelift) after optimization, but only 50% size
  reduction at N=100 — the state-touching functions are duplicated per
  instance.

A `load_dynamic` / `store_dynamic` instruction family would give **shared
mode's size benefits with inlined mode's performance**:

1. **Eliminate toolchain complexity** — no dispatch wrappers, no call graph
   rewriting, no N-branch br_table generation, no per-instance duplication
2. **Eliminate dispatch overhead** — one instruction instead of a function
   call + br_table per memory access
3. **Match baseline performance** — constant memory indices fold to static
   accesses via constant propagation, same as today
4. **Enable the lower-component pipeline** — multiply-instantiated shared
   libraries become trivial to handle in merged binaries
5. **Follow established precedent** — `call` → `call_indirect` for
   functions, `i32.load` → `i32.load_dynamic` for memories

The current tradeoff — choose between binary size (shared) or performance
(inlined) — exists solely because Wasm lacks a dynamic memory index
instruction. A `load_dynamic` instruction would eliminate this tradeoff
entirely.
