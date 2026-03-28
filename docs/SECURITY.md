# Security Comparison: three approaches to accessor design

All three modules expose the string `"hello"` from Module A
to Module B. They differ in how the access boundary is enforced.

### 1. INSECURE — raw i32 index, no bounds check

#### Module A

```wat
(module
  ;; Module A: owns a linear memory with the string "hello"
  ;; Exposes only an accessor function — never the raw memory.
  (memory (export "memory") 1)
  (data (i32.const 0) "hello")

  ;; string_byte(i: i32) -> i32
  ;; Returns the byte at offset i in this module's memory.
  (func (export "string_byte") (param $i i32) (result i32)
    (i32.load8_u (local.get $i))
  )
)
```

#### Module B

```wat
(module
  ;; Module B: has its own linear memory and imports string_byte from A.
  ;; It can read A's data only through the accessor — it never
  ;; receives a pointer into A's memory and has no way to forge one.
  (import "a" "string_byte" (func $string_byte (param i32) (result i32)))

  ;; B's own memory — completely separate from A's
  (memory (export "b_memory") 1)

  ;; read_first() -> i32
  ;; Returns the first byte of A's string via the accessor.
  (func (export "read_first") (result i32)
    (call $string_byte (i32.const 0))
  )

  ;; read_oob() -> i32
  ;; Attempts to read index 10 — beyond "hello" (length 5).
  ;; INSECURE: succeeds, reads whatever is at offset 10 in A's memory.
  (func (export "read_oob") (result i32)
    (call $string_byte (i32.const 10))
  )

  ;; read_at_3() -> i32
  ;; Reads index 3 — still within "hello" (length 5).
  ;; Static index: optimizer can prove 3 < 5 and eliminate the check.
  (func (export "read_at_3") (result i32)
    (call $string_byte (i32.const 3))
  )

  ;; read_at(i: i32) -> i32
  ;; Reads a dynamic index — caller controls which byte to read.
  ;; Dynamic index: optimizer cannot prove safety, bounds check preserved.
  (func (export "read_at") (param $i i32) (result i32)
    (call $string_byte (local.get $i))
  )
)
```

- compiled: `90` + `173` bytes
- merged: `177` bytes

#### After `wasm-merge`

```wat
(module
  (type (;0;) (func (param i32) (result i32)))
  (type (;1;) (func (result i32)))
  (type (;2;) (func (param i32) (result i32)))
  (memory (;0;) 1)
  (memory (;1;) 1)
  (export "b_memory" (memory 0))
  (export "read_first" (func 0))
  (export "read_oob" (func 1))
  (export "read_at_3" (func 2))
  (export "read_at" (func 3))
  (export "memory" (memory 1))
  (export "string_byte" (func 4))
  (func (;0;) (type 1) (result i32)
    i32.const 0
    call 4
  )
  (func (;1;) (type 1) (result i32)
    i32.const 10
    call 4
  )
  (func (;2;) (type 1) (result i32)
    i32.const 3
    call 4
  )
  (func (;3;) (type 0) (param i32) (result i32)
    local.get 0
    call 4
  )
  (func (;4;) (type 2) (param i32) (result i32)
    local.get 0
    i32.load8_u 1
  )
  (data (;0;) (memory 1) (i32.const 0) "hello")
)
```

#### After `wasm-opt -O3 --inlining -O3`

```wat
(module
  (type (;0;) (func (result i32)))
  (type (;1;) (func (param i32) (result i32)))
  (memory (;0;) 1)
  (memory (;1;) 1)
  (export "b_memory" (memory 0))
  (export "read_first" (func 0))
  (export "read_oob" (func 1))
  (export "read_at_3" (func 2))
  (export "read_at" (func 3))
  (export "memory" (memory 1))
  (export "string_byte" (func 3))
  (func (;0;) (type 0) (result i32)
    i32.const 0
    i32.load8_u 1
  )
  (func (;1;) (type 0) (result i32)
    i32.const 10
    i32.load8_u 1
  )
  (func (;2;) (type 0) (result i32)
    i32.const 3
    i32.load8_u 1
  )
  (func (;3;) (type 1) (param i32) (result i32)
    local.get 0
    i32.load8_u 1
  )
  (data (;0;) (memory 1) (i32.const 0) "hello")
)
```

#### Optimized functions

**`read_first`:**

```wat
  (func (;0;) (type 0) (result i32)
    i32.const 0
    i32.load8_u 1
  )
```

**`read_oob`:**

```wat
  (func (;1;) (type 0) (result i32)
    i32.const 10
    i32.load8_u 1
  )
```

**`read_at_3`:**

```wat
  (func (;2;) (type 0) (result i32)
    i32.const 3
    i32.load8_u 1
  )
```

**`read_at`:**

```wat
  (func (;3;) (type 1) (param i32) (result i32)
    local.get 0
    i32.load8_u 1
  )
```


---

### 2. INLINE CHECK — bounds check inside accessor

#### Module A

```wat
(module
  ;; Module A (bounded): owns memory with the string "hello".
  ;; Exports a bounds-checked accessor — B can only read within
  ;; the declared string region, not arbitrary memory.
  (memory (export "memory") 1)
  (data (i32.const 0) "hello")

  ;; Internal knowledge of the string's location and length.
  ;; These globals are not exported — B cannot see or change them.
  (global $str_base i32 (i32.const 0))
  (global $str_len  i32 (i32.const 5))

  ;; string_byte(i: i32) -> i32
  ;; Returns byte at index i within the string, or traps on out-of-bounds.
  (func (export "string_byte") (param $i i32) (result i32)
    ;; Bounds check: i must be < str_len
    (if (i32.ge_u (local.get $i) (global.get $str_len))
      (then (unreachable)))
    (i32.load8_u
      (i32.add (global.get $str_base) (local.get $i)))
  )
)
```

#### Module B

```wat
(module
  ;; Module B (bounded): imports the bounds-checked accessor from A.
  ;; Identical call site as the insecure version — the security
  ;; boundary is enforced entirely within A.
  (import "a" "string_byte" (func $string_byte (param i32) (result i32)))

  (memory (export "b_memory") 1)

  ;; read_first() -> i32
  ;; Returns the first byte of A's string via the accessor.
  (func (export "read_first") (result i32)
    (call $string_byte (i32.const 0))
  )

  ;; read_oob() -> i32
  ;; Attempts to read index 10 — beyond "hello" (length 5).
  ;; SAFE: A's bounds check will trap (unreachable).
  (func (export "read_oob") (result i32)
    (call $string_byte (i32.const 10))
  )

  ;; read_at_3() -> i32
  ;; Reads index 3 — still within "hello" (length 5).
  ;; Static index: optimizer can prove 3 < 5 and eliminate the check.
  (func (export "read_at_3") (result i32)
    (call $string_byte (i32.const 3))
  )

  ;; read_at(i: i32) -> i32
  ;; Reads a dynamic index — A's bounds check enforced at runtime.
  (func (export "read_at") (param $i i32) (result i32)
    (call $string_byte (local.get $i))
  )
)
```

- compiled: `137` + `173` bytes
- merged: `202` bytes

#### After `wasm-merge`

```wat
(module
  (type (;0;) (func (param i32) (result i32)))
  (type (;1;) (func (result i32)))
  (type (;2;) (func (param i32) (result i32)))
  (memory (;0;) 1)
  (memory (;1;) 1)
  (global (;0;) i32 i32.const 0)
  (global (;1;) i32 i32.const 5)
  (export "b_memory" (memory 0))
  (export "read_first" (func 0))
  (export "read_oob" (func 1))
  (export "read_at_3" (func 2))
  (export "read_at" (func 3))
  (export "memory" (memory 1))
  (export "string_byte" (func 4))
  (func (;0;) (type 1) (result i32)
    i32.const 0
    call 4
  )
  (func (;1;) (type 1) (result i32)
    i32.const 10
    call 4
  )
  (func (;2;) (type 1) (result i32)
    i32.const 3
    call 4
  )
  (func (;3;) (type 0) (param i32) (result i32)
    local.get 0
    call 4
  )
  (func (;4;) (type 2) (param i32) (result i32)
    local.get 0
    global.get 1
    i32.ge_u
    if ;; label = @1
      unreachable
    end
    global.get 0
    local.get 0
    i32.add
    i32.load8_u 1
  )
  (data (;0;) (memory 1) (i32.const 0) "hello")
)
```

#### After `wasm-opt -O3 --inlining -O3`

```wat
(module
  (type (;0;) (func (result i32)))
  (type (;1;) (func (param i32) (result i32)))
  (memory (;0;) 1)
  (memory (;1;) 1)
  (export "b_memory" (memory 0))
  (export "read_first" (func 0))
  (export "read_oob" (func 1))
  (export "read_at_3" (func 2))
  (export "read_at" (func 3))
  (export "memory" (memory 1))
  (export "string_byte" (func 3))
  (func (;0;) (type 0) (result i32)
    i32.const 0
    i32.load8_u 1
  )
  (func (;1;) (type 0) (result i32)
    unreachable
  )
  (func (;2;) (type 0) (result i32)
    i32.const 3
    i32.load8_u 1
  )
  (func (;3;) (type 1) (param i32) (result i32)
    local.get 0
    i32.const 5
    i32.ge_u
    if ;; label = @1
      unreachable
    end
    local.get 0
    i32.load8_u 1
  )
  (data (;0;) (memory 1) (i32.const 0) "hello")
)
```

#### Optimized functions

**`read_first`:**

```wat
  (func (;0;) (type 0) (result i32)
    i32.const 0
    i32.load8_u 1
  )
```

**`read_oob`:**

```wat
  (func (;1;) (type 0) (result i32)
    unreachable
  )
```

**`read_at_3`:**

```wat
  (func (;2;) (type 0) (result i32)
    i32.const 3
    i32.load8_u 1
  )
```

**`read_at`:**

```wat
  (func (;3;) (type 1) (param i32) (result i32)
    local.get 0
    i32.const 5
    i32.ge_u
    if ;; label = @1
      unreachable
    end
    local.get 0
    i32.load8_u 1
  )
```


---

### 3. TABLE INDIRECTION — funcref table + bounds check

#### Module A

```wat
(module
  ;; Module A (handle): owns memory with the string "hello".
  ;; Uses a Wasm table for opaque handle-based access.
  ;; B never sees raw pointers — only opaque i32 handle indices.
  ;;
  ;; Each handle is an index into a funcref table. Each entry is a
  ;; bounds-checked accessor for a specific memory region. B cannot
  ;; read the table contents, forge entries, or call arbitrary
  ;; addresses — the table is a first-class Wasm construct with
  ;; runtime bounds checking on every call_indirect.
  (memory (export "memory") 1)
  (data (i32.const 0) "hello")

  ;; ── Accessor function type ──────────────────────────────────────
  (type $accessor (func (param i32) (result i32)))

  ;; ── Resource table (Wasm table of funcref) ──────────────────────
  ;; Each slot holds a function that knows its own region's bounds.
  (table 1 funcref)
  (elem (i32.const 0) $r0_byte)

  ;; Accessor for handle 0: "hello" at base=0, length=5
  (func $r0_byte (param $i i32) (result i32)
    (if (i32.ge_u (local.get $i) (i32.const 5))
      (then (unreachable)))
    (i32.load8_u
      (i32.add (i32.const 0) (local.get $i)))
  )

  ;; get_byte(handle, i) -> byte
  ;; Dispatches through the Wasm table via call_indirect.
  ;; The table bounds-checks the handle index automatically —
  ;; an invalid handle traps before any code runs.
  (func (export "get_byte") (param $handle i32) (param $i i32) (result i32)
    (call_indirect (type $accessor)
      (local.get $i)
      (local.get $handle))
  )
)
```

#### Module B

```wat
(module
  ;; Module B (handle): imports handle-based accessor from A.
  ;; B never sees raw memory addresses — it receives an opaque handle
  ;; index and passes it to get_byte.
  ;;
  ;; In the full component model, the handle would be passed to B
  ;; at the call boundary during lift/lower. Here we use a constant
  ;; handle (0) that refers to the pre-registered resource in A's
  ;; static table. B cannot read the table or forge a handle that
  ;; bypasses bounds checks — A's memory is in a separate address
  ;; space (memory 1 after merge).
  (import "a" "get_byte" (func $get_byte (param i32 i32) (result i32)))

  (memory (export "b_memory") 1)

  ;; read_first() -> i32
  ;; Reads byte 0 from the resource at handle 0.
  (func (export "read_first") (result i32)
    (call $get_byte (i32.const 0) (i32.const 0))
  )

  ;; read_oob() -> i32
  ;; Attempts to read index 10 from handle 0 — beyond "hello" (length 5).
  ;; SAFE: A's bounds check will trap (unreachable).
  (func (export "read_oob") (result i32)
    (call $get_byte (i32.const 0) (i32.const 10))
  )

  ;; read_at_3() -> i32
  ;; Reads index 3 from handle 0 — still within "hello" (length 5).
  ;; Static index: optimizer can prove 3 < 5 and eliminate the check.
  (func (export "read_at_3") (result i32)
    (call $get_byte (i32.const 0) (i32.const 3))
  )

  ;; read_at(i: i32) -> i32
  ;; Reads a dynamic index from handle 0 — A's bounds check enforced at runtime.
  (func (export "read_at") (param $i i32) (result i32)
    (call $get_byte (i32.const 0) (local.get $i))
  )
)
```

- compiled: `169` + `181` bytes
- merged: `232` bytes

#### After `wasm-merge`

```wat
(module
  (type (;0;) (func (param i32 i32) (result i32)))
  (type (;1;) (func (result i32)))
  (type (;2;) (func (param i32) (result i32)))
  (type (;3;) (func (param i32) (result i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (table (;0;) 1 funcref)
  (memory (;0;) 1)
  (memory (;1;) 1)
  (export "b_memory" (memory 0))
  (export "read_first" (func 0))
  (export "read_oob" (func 1))
  (export "read_at_3" (func 2))
  (export "read_at" (func 3))
  (export "memory" (memory 1))
  (export "get_byte" (func 5))
  (elem (;0;) (i32.const 0) func 4)
  (func (;0;) (type 1) (result i32)
    i32.const 0
    i32.const 0
    call 5
  )
  (func (;1;) (type 1) (result i32)
    i32.const 0
    i32.const 10
    call 5
  )
  (func (;2;) (type 1) (result i32)
    i32.const 0
    i32.const 3
    call 5
  )
  (func (;3;) (type 2) (param i32) (result i32)
    i32.const 0
    local.get 0
    call 5
  )
  (func (;4;) (type 3) (param i32) (result i32)
    local.get 0
    i32.const 5
    i32.ge_u
    if ;; label = @1
      unreachable
    end
    i32.const 0
    local.get 0
    i32.add
    i32.load8_u 1
  )
  (func (;5;) (type 4) (param i32 i32) (result i32)
    local.get 1
    local.get 0
    call_indirect (type 3)
  )
  (data (;0;) (memory 1) (i32.const 0) "hello")
)
```

#### After `wasm-opt -O3 --inlining -O3`

```wat
(module
  (type (;0;) (func (result i32)))
  (type (;1;) (func (param i32) (result i32)))
  (type (;2;) (func (param i32 i32) (result i32)))
  (table (;0;) 1 funcref)
  (memory (;0;) 1)
  (memory (;1;) 1)
  (export "b_memory" (memory 0))
  (export "read_first" (func 0))
  (export "read_oob" (func 1))
  (export "read_at_3" (func 2))
  (export "read_at" (func 3))
  (export "memory" (memory 1))
  (export "get_byte" (func 4))
  (elem (;0;) (i32.const 0) func 3)
  (func (;0;) (type 0) (result i32)
    i32.const 0
    i32.load8_u 1
  )
  (func (;1;) (type 0) (result i32)
    unreachable
  )
  (func (;2;) (type 0) (result i32)
    i32.const 3
    i32.load8_u 1
  )
  (func (;3;) (type 1) (param i32) (result i32)
    local.get 0
    i32.const 5
    i32.ge_u
    if ;; label = @1
      unreachable
    end
    local.get 0
    i32.load8_u 1
  )
  (func (;4;) (type 2) (param i32 i32) (result i32)
    local.get 1
    local.get 0
    call_indirect (type 1)
  )
  (data (;0;) (memory 1) (i32.const 0) "hello")
)
```

#### Optimized functions

**`read_first`:**

```wat
  (func (;0;) (type 0) (result i32)
    i32.const 0
    i32.load8_u 1
  )
```

**`read_oob`:**

```wat
  (func (;1;) (type 0) (result i32)
    unreachable
  )
```

**`read_at_3`:**

```wat
  (func (;2;) (type 0) (result i32)
    i32.const 3
    i32.load8_u 1
  )
```

**`read_at`:**

```wat
  (func (;3;) (type 1) (param i32) (result i32)
    local.get 0
    i32.const 5
    i32.ge_u
    if ;; label = @1
      unreachable
    end
    local.get 0
    i32.load8_u 1
  )
```


---

## Comparison — optimized functions across all three approaches

### `read_first`

**INSECURE:**

```wat
  (func (;0;) (type 0) (result i32)
    i32.const 0
    i32.load8_u 1
  )
```

**INLINE CHECK:**

```wat
  (func (;0;) (type 0) (result i32)
    i32.const 0
    i32.load8_u 1
  )
```

**TABLE INDIRECTION:**

```wat
  (func (;0;) (type 0) (result i32)
    i32.const 0
    i32.load8_u 1
  )
```

### `read_oob`

**INSECURE:**

```wat
  (func (;1;) (type 0) (result i32)
    i32.const 10
    i32.load8_u 1
  )
```

**INLINE CHECK:**

```wat
  (func (;1;) (type 0) (result i32)
    unreachable
  )
```

**TABLE INDIRECTION:**

```wat
  (func (;1;) (type 0) (result i32)
    unreachable
  )
```

### `read_at_3`

**INSECURE:**

```wat
  (func (;2;) (type 0) (result i32)
    i32.const 3
    i32.load8_u 1
  )
```

**INLINE CHECK:**

```wat
  (func (;2;) (type 0) (result i32)
    i32.const 3
    i32.load8_u 1
  )
```

**TABLE INDIRECTION:**

```wat
  (func (;2;) (type 0) (result i32)
    i32.const 3
    i32.load8_u 1
  )
```

### `read_at`

**INSECURE:**

```wat
  (func (;3;) (type 1) (param i32) (result i32)
    local.get 0
    i32.load8_u 1
  )
```

**INLINE CHECK:**

```wat
  (func (;3;) (type 1) (param i32) (result i32)
    local.get 0
    i32.const 5
    i32.ge_u
    if ;; label = @1
      unreachable
    end
    local.get 0
    i32.load8_u 1
  )
```

**TABLE INDIRECTION:**

```wat
  (func (;3;) (type 1) (param i32) (result i32)
    local.get 0
    i32.const 5
    i32.ge_u
    if ;; label = @1
      unreachable
    end
    local.get 0
    i32.load8_u 1
  )
```

## Summary

**INSECURE:** `string_byte(i)` loads any byte from A's memory — B can pass
any i32 value and read A's entire address space.

**INLINE CHECK:** A knows the string bounds internally (`base=0`, `len=5`) and
traps on out-of-bounds access. B's call site is unchanged, but the security
boundary is enforced within A. The bounds check is visible in the exported
accessor (`i32.ge_u` + `unreachable`) and survives optimization for dynamic
callers.

**TABLE INDIRECTION:** B receives an opaque handle that indexes into a Wasm
`funcref` table. Each table slot holds a bounds-checked accessor for a specific
memory region. `get_byte(handle, i)` dispatches via `call_indirect` — the Wasm
runtime bounds-checks the handle automatically, and each accessor bounds-checks
the byte index. B cannot read the table, forge entries, or call arbitrary
addresses. The table is a first-class Wasm construct, not a data structure in
linear memory.

### What the optimizer does

| Function | Index | Insecure | Inline check | Table indirection |
|----------|-------|----------|--------------|-------------------|
| `read_first()` | `0` (static) | direct load | check eliminated (0 < 5) | check eliminated |
| `read_oob()` | `10` (static) | direct load (reads garbage) | reduced to `unreachable` (10 >= 5) | reduced to `unreachable` |
| `read_at_3()` | `3` (static) | direct load | check eliminated (3 < 5) | check eliminated |
| `read_at(i)` | dynamic | direct load | **bounds check preserved** | **bounds check preserved** |

For static indices, the optimizer proves safety (or unsafety) at compile time.
For dynamic indices, the bounds check must survive — it's the runtime safety net.

The insecure version produces a bare `i32.load8_u` in all cases — fast, but
no safety at any level.

**Zero-cost abstraction: safety at the source level, bare metal at runtime
for provably-safe accesses, minimal overhead for dynamic ones.**
