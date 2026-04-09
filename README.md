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

### Example: reading a byte

```c
// module_a.c — compiled to a.wasm
static char data[] = "hello";

// Accessor: B can read bytes but never gets a pointer into A's memory
int string_byte(int i) {
    if (i < 0 || i >= 5) __builtin_trap();
    return data[i];
}
```

```c
// module_b.c — compiled to b.wasm, imports string_byte from A
int string_byte(int i);  // cross-module import

int read_first() {
    return string_byte(0);
}
```

After `wasm-merge` + `wasm-opt`, the merged code behaves as if:

```c
// merged — conceptual C equivalent of what the optimizer produces
static char a_data[] = "hello";  // A's memory (memory 1)

int read_first() {
    return a_data[0];  // direct load — no call, no bounds check
}
```

The function call, the bounds check (0 < 5 is provably true), and the module boundary are all eliminated. A dynamic index like `string_byte(i)` preserves the bounds check — the optimizer only removes what it can prove safe.

### Example: calling toupper from a shared libc

```c
// libc (shared library) — standard C implementation
#include <ctype.h>
int toupper(int c) {
    return (c >= 'a' && c <= 'z') ? c - 32 : c;
}

void* malloc(size_t size);  // allocator with per-instance heap
void  free(void* ptr);
```

```c
// module_b.c — imports toupper and malloc from libc
#include <ctype.h>
#include <stdlib.h>

char* uppercase(const char* src, int len) {
    char* dst = malloc(len + 1);
    for (int i = 0; i < len; i++)
        dst[i] = toupper(src[i]);       // cross-module call per character
    dst[len] = '\0';
    return dst;
}

int main() {
    char* result = uppercase("hello", 5);  // → "HELLO"
    free(result);
}
```

After merge + optimize, `toupper` is inlined and `malloc` uses B's own heap:

```c
// merged + optimized — conceptual C equivalent
int main() {
    // inlined malloc on B's memory
    char* dst = &inst1_heap[inst1_heap_end];
    inst1_heap_end += 6;

    // inlined uppercase with inlined toupper
    const char* src = "hello";
    for (int i = 0; i < 5; i++)
        dst[i] = (src[i] >= 'a' && src[i] <= 'z')
                 ? src[i] - 32 : src[i];
    dst[5] = '\0';
    // dst → "HELLO"

    // inlined free (bump allocator — nop)
}
```

All three accessor calls inline to direct memory operations on A's memory. B never receives a pointer — it cannot read outside the declared region. The same pattern applies to `malloc`/`free` calls through a shared library.

## Shared library linking

N consumer modules can share a single library (e.g. wasi-libc) while maintaining independent per-instance state. Two modes:

- **Inlined** (`--specialize`): per-consumer specialized copies, static memory indices. After `wasm-opt -O4` — **faster than baseline**.
- **Shared** (`--dispatch`): `br_table` dispatch, one copy of function bodies — **smallest binary** (-98% at N=100).

### Example: two apps sharing wasi-libc

```c
// app_a.c — consumer 0
#include <stdlib.h>
void* alloc_a(int size) { return malloc(size); }

// app_b.c — consumer 1
#include <stdlib.h>
void* alloc_b(int size) { return malloc(size); }
```

Without merging: ship 3 modules (app_a.wasm, app_b.wasm, libc.wasm) — runtime instantiates libc twice, no cross-module inlining. Or duplicate libc into each consumer — O(N) binary size.

```bash
# Merge all three into one module:
wasm-merge app_a.wasm=inst0 app_b.wasm=inst1 libc.wasm=lib --specialize --lib lib -o merged.wasm
```

After merge, each consumer gets its own memory and globals. The library's `malloc` is shared — one copy of the function body with a per-consumer memory selector:

```c
// wasi-libc (the shared library) — original, unmodified
static char heap[...];
static int  heap_end;

void* malloc(int size) {
    int old = heap_end;
    heap_end += align_up(size, 8);
    return &heap[old];
}
```

Compiled to Wasm, the memory index is a static immediate — baked into the bytecode:

```wat
(module
  (memory (export "memory") 1)
  (global $heap_end (mut i32) (i32.const 0))

  (func (export "malloc") (param $size i32) (result i32)
    (local $old i32)
    (local.set $old (global.get $heap_end))
    (global.set $heap_end
      (i32.add (global.get $heap_end)
               (call $align_up (local.get $size) (i32.const 8))))
    (local.get $old))

  (func $align_up (param $n i32) (param $a i32) (result i32)
    (i32.and
      (i32.add (local.get $n) (i32.sub (local.get $a) (i32.const 1)))
      (i32.sub (i32.const 0) (local.get $a))))
)
```

One memory, one global. A second instance needs a second module — there is no way for this function body to reference a different memory or global at runtime.

After merge, the module has two memories and two globals (one per consumer). `malloc` is rewritten with an `$instance` parameter and `br_table` dispatch to select the correct memory and global. Shell wrappers hardwire the instance index:

```wat
(module
  (memory (;0;) 1)                       ;; app_a's heap
  (memory (;1;) 1)                       ;; app_b's heap
  (global (;0;) (mut i32) i32.const 0)   ;; inst0_heap_end
  (global (;1;) (mut i32) i32.const 0)   ;; inst1_heap_end

  ;; Shared malloc — one copy, dispatches on $instance at runtime
  (func $malloc (param $size i32) (param $instance i32) (result i32)
    (local $old i32)
    ;; old = heap_end[$instance]  (via br_table)
    (block $done
      (block $b1 (block $b0
        (br_table $b0 $b1 (local.get $instance)))
        (local.set $old (global.get 0)) (br $done))             ;; instance 0
      (local.set $old (global.get 1)))                          ;; instance 1
    ;; heap_end[$instance] += align_up(size, 8)
    (block $done2
      (block $b1 (block $b0
        (br_table $b0 $b1 (local.get $instance)))
        (global.set 0 (i32.add (global.get 0)
          (call $align_up (local.get $size) (i32.const 8))))
        (br $done2))
      (global.set 1 (i32.add (global.get 1)
        (call $align_up (local.get $size) (i32.const 8)))))
    (local.get $old))

  ;; align_up — pure function, shared as-is (no dispatch needed)
  (func $align_up (param $n i32) (param $a i32) (result i32)
    (i32.and
      (i32.add (local.get $n) (i32.sub (local.get $a) (i32.const 1)))
      (i32.sub (i32.const 0) (local.get $a))))

  ;; Shell wrappers — hardwire the instance index
  (func $alloc_a (export "alloc_a") (param $size i32) (result i32)
    (call $malloc (local.get $size) (i32.const 0)))
  (func $alloc_b (export "alloc_b") (param $size i32) (result i32)
    (call $malloc (local.get $size) (i32.const 1)))
)
```

A main function calling both allocators:

```c
void _start() {
    void* a = alloc_a(64);   // → malloc(64, 0)
    void* b = alloc_b(64);   // → malloc(64, 1)
}
```

After `wasm-opt -O4`, the constant `$instance` is propagated through `malloc`, the `br_table` is eliminated, and the shell wrappers are inlined:

```wat
(func $_start (export "_start")
  (local $old_a i32) (local $old_b i32)
  ;; alloc_a(64) → malloc(64, 0) → branch 0 folded
  (local.set $old_a (global.get 0))
  (global.set 0 (i32.add (global.get 0) (i32.const 64)))
  ;; alloc_b(64) → malloc(64, 1) → branch 1 folded
  (local.set $old_b (global.get 1))
  (global.set 1 (i32.add (global.get 1) (i32.const 64))))
```

No call, no `br_table`, no instance parameter. Each allocation is a direct global read + update against the correct per-consumer heap pointer.

App A and B are fully isolated — each has its own memory and allocator state — but share one copy of `align_up` and run in a single module with no cross-module call overhead.

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
