# wasm-merge

A multi-memory module merger and shared library linker for WebAssembly.

## What it does

`wasm-merge` takes multiple Wasm modules and produces a single merged module
where each original module retains its own linear memory (multi-memory).
Cross-module imports are resolved, function indices remapped, and the result
is one flat `.wasm` file.

## Modes

### Basic merge

Merges modules with multi-memory, resolving imports by label matching:

```bash
wasm-merge consumer.wasm=app library.wasm=lib -o merged.wasm
```

Each module keeps its own memory. Imports from `"lib"` resolve to the
library's exports. Unresolved imports (e.g., WASI) pass through.

### Inlined merge (`--specialize --lib <label>`)

Merges a shared library with N consumer modules, creating per-instance
specialized copies of state-touching functions with the correct memory
and global indices baked in. After `wasm-opt -O4`, shell wrappers are
inlined — **faster than baseline**:

```bash
wasm-merge \
  app_a.wasm=inst0 app_b.wasm=inst1 wasi-libc.wasm=lib \
  --specialize --lib lib \
  -o merged.wasm
```

**What happens internally:**

1. Analyzes the library's call graph to identify state-touching functions
   (any function that transitively accesses memory or mutable globals)
2. For each consumer, traces which library exports it imports and which
   state-touching functions are reachable from those imports
3. Creates specialized copies of only the reachable functions, with the
   consumer's memory index and global copy baked into every instruction
4. Pure functions (no memory/global access) are shared — one copy for all
5. Shell wrappers at the export boundary preserve the original API
6. Consumers import `malloc(size)` with the original signature — no
   instance indices, no special naming

**Key properties:**

- **Zero dispatch overhead**: no `br_table`, no runtime dispatch. Memory
  instructions use static indices (`i32.load memory=3`) directly.
- **Per-consumer reachability**: only duplicates the functions each consumer
  actually calls, not the entire library.
- **Hardware isolation**: each consumer has its own memory — one instance
  cannot address another's memory regardless of bugs.
- **Original API**: consumers import standard function names. The merge tool
  handles instance assignment automatically.

### Component merge

Merges a binary component (nested core modules) into a flat module:

```bash
wasm-merge component.wasm -o merged.wasm
```

### Options

- `--exports-from <pos>`: only re-export from the module at the given
  position. Combined with `wasm-opt --remove-unused-module-elements`,
  this enables dead code elimination of unused instances.
- `--specialize --lib <label>`: enable per-instance inlined mode for
  the module with the given label.
- `--dispatch --lib <label>`: enable shared mode with `br_table` dispatch.
- `--verify`: post-merge isolation check. Validates every function only
  accesses its allowed memory indices. Exits with error if violations found.

## How inlined mode works

Given a library with this structure:

```c
// wasi-libc internals
static int __heap_end = 4096;

void* sbrk(int n) {
    void* p = (void*)__heap_end;  // reads memory + global
    __heap_end += n;               // writes global
    return p;
}

void* malloc(int size) {
    return sbrk(align_up(size, 8));  // calls state-touching function
}

int strlen(const char* s) {         // reads memory (state-touching)
    int n = 0; while (s[n]) n++; return n;
}

int align_up(int n, int a) {       // pure arithmetic — shared
    return (n + a - 1) & ~(a - 1);
}
```

For two consumers (app_a imports malloc, app_b imports malloc+strlen):

```
Output module:
  memory 0: app_a's memory
  memory 1: app_b's memory
  global 0: __heap_end for app_a
  global 1: __heap_end for app_b

  align_up()          — 1 copy, shared (pure)
  malloc_for_inst0()  — uses memory 0, global 0
  sbrk_for_inst0()    — uses memory 0, global 0
  malloc_for_inst1()  — uses memory 1, global 1
  sbrk_for_inst1()    — uses memory 1, global 1
  strlen_for_inst1()  — uses memory 1 (only app_b imports it)

  shell wrapper: "malloc" for app_a → calls malloc_for_inst0
  shell wrapper: "malloc" for app_b → calls malloc_for_inst1
  shell wrapper: "strlen" for app_b → calls strlen_for_inst1
```

After `wasm-opt --remove-unused-module-elements`, any unreachable
specialized copies are stripped.

## Shared mode (`--dispatch`)

The `--dispatch` flag provides an alternative approach that uses `br_table`
dispatch wrappers to select the correct memory at runtime. This keeps one
copy of each function body but adds overhead per memory access.

```bash
wasm-merge a.wasm=inst0 b.wasm=inst1 lib.wasm=lib --dispatch --lib lib -o merged.wasm
```

| | Shared (`--dispatch`) | Inlined (`--specialize`) |
|---|---|---|
| Binary size (N=50) | 26 KB (**-97%**) | 414 KB (**-49%**) |
| V8 overhead | +176% | **-25%** |
| Cranelift overhead | +2954% | **-16%** |
| Function bodies | 1 copy (shared) | N copies (per-instance) |
| Pure functions | Shared | Shared |
| Dispatch mechanism | `br_table` per memory access | None (static indices) |
| Memory isolation | ✓ (multi-memory) | ✓ (multi-memory) |

**Shared** is best for size-sensitive deployments (browsers, edge).
**Inlined** is best for performance-sensitive deployments (servers).

## Building

```bash
cd wasm-merge && cargo build --release
```

## Benchmarks

```bash
scripts/benchmark-shared-library.sh && node scripts/bench-format.mjs
```

Generates `output/bench.json` and `docs/BENCHMARK.md` with size and performance
tables across N=2 to N=100 consumers, tested on V8 and Wasmtime.
