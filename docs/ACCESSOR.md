# Shared Memory Accessor Patterns

Module A owns a linear memory region. Module B accesses it only through
exported accessor functions — never via a raw pointer. After merge and
optimization, accessor calls are **completely eliminated** for provably-safe
accesses — zero-cost abstraction.

Two dimensions are demonstrated:
1. **Security** — how the access boundary is enforced (none, bounds check, opaque handle)
2. **Ownership** — who can read, write, or transfer the region (borrow, mutable borrow, move)

All patterns work today with multi-memory, `wasm-merge`, and `wasm-opt` — no spec changes needed.

## Part 1: Security — accessor design comparison

All three variants expose the string `"hello"` from Module A
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

### Security — side-by-side comparison

#### `read_first`

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

#### `read_oob`

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

#### `read_at_3`

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

#### `read_at`

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

### Security summary

| Function | Index | Insecure | Inline check | Table indirection |
|----------|-------|----------|--------------|-------------------|
| `read_first()` | `0` (static) | direct load | check eliminated (0 < 5) | check eliminated |
| `read_oob()` | `10` (static) | direct load (reads garbage) | reduced to `unreachable` (10 >= 5) | reduced to `unreachable` |
| `read_at_3()` | `3` (static) | direct load | check eliminated (3 < 5) | check eliminated |
| `read_at(i)` | dynamic | direct load | **bounds check preserved** | **bounds check preserved** |

For static indices, the optimizer proves safety at compile time. For dynamic
indices, the bounds check survives — the runtime safety net.

## Part 2: Ownership — borrow and move semantics

Three patterns showing how Rust-inspired ownership semantics can be
enforced at the module boundary using accessor functions.

### 4. READ-ONLY BORROW — read accessor only, no write

#### Module A
```wat
(module
  ;; Module A (read-only borrow): owns memory with "hello world".
  ;; Exports only a read accessor — B cannot modify the data.
  (memory (export "memory") 1)
  (data (i32.const 0) "hello world")

  (global $base i32 (i32.const 0))
  (global $len  i32 (i32.const 11))

  (func (export "region_len") (result i32)
    (global.get $len))

  ;; Read-only: returns byte at index i, traps if out of bounds.
  ;; No write accessor exists — B has no mechanism to modify memory.
  (func (export "region_read") (param $i i32) (result i32)
    (if (i32.ge_u (local.get $i) (global.get $len))
      (then (unreachable)))
    (i32.load8_u (i32.add (global.get $base) (local.get $i))))
)
```

#### Module B
```wat
(module
  ;; Module B (read-only borrow consumer): reads A's region.
  ;; Cannot modify A's memory — no write accessor available.
  (import "a" "region_len"  (func $len  (result i32)))
  (import "a" "region_read" (func $read (param i32) (result i32)))

  (memory (export "b_memory") 1)

  ;; count_vowels() -> i32: counts vowels in A's region via read accessor.
  (func (export "count_vowels") (result i32)
    (local $i i32) (local $n i32) (local $c i32) (local $end i32)
    (local.set $end (call $len))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $end)))
        (local.set $c (call $read (local.get $i)))
        (if (i32.or
              (i32.or (i32.eq (local.get $c) (i32.const 97))   ;; a
                      (i32.eq (local.get $c) (i32.const 101))) ;; e
              (i32.or (i32.or (i32.eq (local.get $c) (i32.const 105))   ;; i
                              (i32.eq (local.get $c) (i32.const 111)))  ;; o
                      (i32.eq (local.get $c) (i32.const 117))))         ;; u
          (then (local.set $n (i32.add (local.get $n) (i32.const 1)))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.get $n))
)
```

- merged: `252` bytes

#### After `wasm-merge`
```wat
(module
  (type (;0;) (func (result i32)))
  (type (;1;) (func (param i32) (result i32)))
  (type (;2;) (func (result i32)))
  (type (;3;) (func (param i32) (result i32)))
  (memory (;0;) 1)
  (memory (;1;) 1)
  (global (;0;) i32 i32.const 0)
  (global (;1;) i32 i32.const 11)
  (export "b_memory" (memory 0))
  (export "count_vowels" (func 0))
  (export "memory" (memory 1))
  (export "region_len" (func 1))
  (export "region_read" (func 2))
  (func (;0;) (type 0) (result i32)
    (local i32 i32 i32 i32)
    call 1
    local.set 3
    block ;; label = @1
      loop ;; label = @2
        local.get 0
        local.get 3
        i32.ge_u
        br_if 1 (;@1;)
        local.get 0
        call 2
        local.set 2
        local.get 2
        i32.const 97
        i32.eq
        local.get 2
        i32.const 101
        i32.eq
        i32.or
        local.get 2
        i32.const 105
        i32.eq
        local.get 2
        i32.const 111
        i32.eq
        i32.or
        local.get 2
        i32.const 117
        i32.eq
        i32.or
        i32.or
        if ;; label = @3
          local.get 1
          i32.const 1
          i32.add
          local.set 1
        end
        local.get 0
        i32.const 1
        i32.add
        local.set 0
        br 0 (;@2;)
      end
    end
    local.get 1
  )
  (func (;1;) (type 2) (result i32)
    global.get 1
  )
  (func (;2;) (type 3) (param i32) (result i32)
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
  (data (;0;) (memory 1) (i32.const 0) "hello world")
)
```

#### After `wasm-opt -O3 --inlining -O3`

**`count_vowels`:**
```wat
  (func (;0;) (type 0) (result i32)
    (local i32 i32)
    loop ;; label = @1
      local.get 1
      i32.const 11
      i32.lt_u
      if ;; label = @2
        local.get 1
        i32.const 11
        i32.ge_u
        if ;; label = @3
          unreachable
        end
        local.get 0
        i32.const 1
        i32.add
        local.get 0
        local.get 1
        i32.load8_u 1
        local.tee 0
        i32.const 97
        i32.eq
        local.get 0
        i32.const 101
        i32.eq
        i32.or
        local.get 0
        i32.const 105
        i32.eq
        local.get 0
        i32.const 111
        i32.eq
        i32.or
        local.get 0
        i32.const 117
        i32.eq
        i32.or
        i32.or
        select
        local.set 0
        local.get 1
        i32.const 1
        i32.add
        local.set 1
        br 1 (;@1;)
      end
    end
    local.get 0
  )
```

---

### 5. MUTABLE BORROW — read + write accessors, call-scoped

#### Module A
```wat
(module
  ;; Module A (mutable borrow): owns memory with "hello world".
  ;; Exports read AND write accessors — B can modify within bounds.
  ;; The borrow is call-scoped: synchronous Wasm guarantees mutual exclusion.
  (memory (export "memory") 1)
  (data (i32.const 0) "hello world")

  (global $base i32 (i32.const 0))
  (global $len  i32 (i32.const 11))

  (func (export "region_len") (result i32)
    (global.get $len))

  (func (export "region_read") (param $i i32) (result i32)
    (if (i32.ge_u (local.get $i) (global.get $len))
      (then (unreachable)))
    (i32.load8_u (i32.add (global.get $base) (local.get $i))))

  ;; Write accessor — B can modify bytes within the bounded region.
  (func (export "region_write") (param $i i32) (param $val i32)
    (if (i32.ge_u (local.get $i) (global.get $len))
      (then (unreachable)))
    (i32.store8 (i32.add (global.get $base) (local.get $i)) (local.get $val)))
)
```

#### Module B
```wat
(module
  ;; Module B (mutable borrow consumer): reads and writes A's region.
  ;; Uppercases "hello world" in-place via accessor functions.
  (import "a" "region_len"   (func $len   (result i32)))
  (import "a" "region_read"  (func $read  (param i32) (result i32)))
  (import "a" "region_write" (func $write (param i32 i32)))

  (memory (export "b_memory") 1)

  ;; uppercase(): converts A's region to uppercase in-place.
  (func (export "uppercase")
    (local $i i32) (local $c i32) (local $end i32)
    (local.set $end (call $len))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $end)))
        (local.set $c (call $read (local.get $i)))
        (if (i32.and
              (i32.ge_u (local.get $c) (i32.const 97))
              (i32.le_u (local.get $c) (i32.const 122)))
          (then
            (call $write (local.get $i) (i32.sub (local.get $c) (i32.const 32)))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop))))

  ;; read_back(i) -> i32: reads byte i from A to verify mutation persisted.
  (func (export "read_back") (param $i i32) (result i32)
    (call $read (local.get $i)))
)
```

- merged: `300` bytes

#### After `wasm-merge`
```wat
(module
  (type (;0;) (func (result i32)))
  (type (;1;) (func (param i32) (result i32)))
  (type (;2;) (func (param i32 i32)))
  (type (;3;) (func))
  (type (;4;) (func (result i32)))
  (type (;5;) (func (param i32) (result i32)))
  (type (;6;) (func (param i32 i32)))
  (memory (;0;) 1)
  (memory (;1;) 1)
  (global (;0;) i32 i32.const 0)
  (global (;1;) i32 i32.const 11)
  (export "b_memory" (memory 0))
  (export "uppercase" (func 0))
  (export "read_back" (func 1))
  (export "memory" (memory 1))
  (export "region_len" (func 2))
  (export "region_read" (func 3))
  (export "region_write" (func 4))
  (func (;0;) (type 3)
    (local i32 i32 i32)
    call 2
    local.set 2
    block ;; label = @1
      loop ;; label = @2
        local.get 0
        local.get 2
        i32.ge_u
        br_if 1 (;@1;)
        local.get 0
        call 3
        local.set 1
        local.get 1
        i32.const 97
        i32.ge_u
        local.get 1
        i32.const 122
        i32.le_u
        i32.and
        if ;; label = @3
          local.get 0
          local.get 1
          i32.const 32
          i32.sub
          call 4
        end
        local.get 0
        i32.const 1
        i32.add
        local.set 0
        br 0 (;@2;)
      end
    end
  )
  (func (;1;) (type 1) (param i32) (result i32)
    local.get 0
    call 3
  )
  (func (;2;) (type 4) (result i32)
    global.get 1
  )
  (func (;3;) (type 5) (param i32) (result i32)
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
  (func (;4;) (type 6) (param i32 i32)
    local.get 0
    global.get 1
    i32.ge_u
    if ;; label = @1
      unreachable
    end
    global.get 0
    local.get 0
    i32.add
    local.get 1
    i32.store8 1
  )
  (data (;0;) (memory 1) (i32.const 0) "hello world")
)
```

#### After `wasm-opt -O3 --inlining -O3`

**`uppercase`:**
```wat
  (func (;0;) (type 0)
    (local i32 i32)
    loop ;; label = @1
      local.get 0
      i32.const 11
      i32.lt_u
      if ;; label = @2
        local.get 0
        i32.const 11
        i32.ge_u
        if ;; label = @3
          unreachable
        end
        local.get 0
        i32.load8_u 1
        local.tee 1
        i32.const 122
        i32.le_u
        local.get 1
        i32.const 97
        i32.ge_u
        i32.and
        if ;; label = @3
          local.get 0
          i32.const 11
          i32.ge_u
          if ;; label = @4
            unreachable
          end
          local.get 0
          local.get 1
          i32.const 32
          i32.sub
          i32.store8 1
        end
        local.get 0
        i32.const 1
        i32.add
        local.set 0
        br 1 (;@1;)
      end
    end
  )
```

**`read_back`:**
```wat
  (func (;1;) (type 1) (param i32) (result i32)
    local.get 0
    i32.const 11
    i32.ge_u
    if ;; label = @1
      unreachable
    end
    local.get 0
    i32.load8_u 1
  )
```

---

### 6. MOVE — read + release, use-after-move traps

#### Module A
```wat
(module
  ;; Module A (move/transfer): owns memory with "secret!".
  ;; After release(), all accessors trap — ownership has been transferred.
  (memory (export "memory") 1)
  (data (i32.const 0) "secret!")

  (global $len   i32 (i32.const 7))
  (global $alive (mut i32) (i32.const 1))  ;; 1 = owned, 0 = moved

  (func (export "region_len") (result i32)
    (if (i32.eqz (global.get $alive)) (then (unreachable)))
    (global.get $len))

  ;; Read accessor — traps after ownership transfer.
  (func (export "region_read") (param $i i32) (result i32)
    (if (i32.eqz (global.get $alive)) (then (unreachable)))
    (if (i32.ge_u (local.get $i) (global.get $len)) (then (unreachable)))
    (i32.load8_u (local.get $i)))

  ;; Release: permanently transfers ownership. All accessors trap after this.
  (func (export "release")
    (if (i32.eqz (global.get $alive)) (then (unreachable)))
    (global.set $alive (i32.const 0)))
)
```

#### Module B
```wat
(module
  ;; Module B (move consumer): takes ownership of A's region.
  ;; Copies data into B's memory via accessor, then calls release.
  ;; After release, A's accessors trap — use-after-move is caught at runtime.
  (import "a" "region_len"  (func $len     (result i32)))
  (import "a" "region_read" (func $read    (param i32) (result i32)))
  (import "a" "release"     (func $release))

  (memory (export "b_memory") 1)

  ;; take_data() -> i32: copies A's region into B's memory, releases A.
  ;; Returns the length of the copied data.
  (func (export "take_data") (result i32)
    (local $i i32) (local $n i32)
    (local.set $n (call $len))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (i32.store8 (local.get $i) (call $read (local.get $i)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (call $release)
    (local.get $n))

  ;; read_after_move() -> i32: attempts to read from A after transfer — traps.
  (func (export "read_after_move") (result i32)
    (call $read (i32.const 0)))
)
```

- merged: `274` bytes

#### After `wasm-merge`
```wat
(module
  (type (;0;) (func (result i32)))
  (type (;1;) (func (param i32) (result i32)))
  (type (;2;) (func))
  (type (;3;) (func (result i32)))
  (type (;4;) (func (param i32) (result i32)))
  (type (;5;) (func))
  (memory (;0;) 1)
  (memory (;1;) 1)
  (global (;0;) i32 i32.const 7)
  (global (;1;) (mut i32) i32.const 1)
  (export "b_memory" (memory 0))
  (export "take_data" (func 0))
  (export "read_after_move" (func 1))
  (export "memory" (memory 1))
  (export "region_len" (func 2))
  (export "region_read" (func 3))
  (export "release" (func 4))
  (func (;0;) (type 0) (result i32)
    (local i32 i32)
    call 2
    local.set 1
    block ;; label = @1
      loop ;; label = @2
        local.get 0
        local.get 1
        i32.ge_u
        br_if 1 (;@1;)
        local.get 0
        local.get 0
        call 3
        i32.store8
        local.get 0
        i32.const 1
        i32.add
        local.set 0
        br 0 (;@2;)
      end
    end
    call 4
    local.get 1
  )
  (func (;1;) (type 0) (result i32)
    i32.const 0
    call 3
  )
  (func (;2;) (type 3) (result i32)
    global.get 1
    i32.eqz
    if ;; label = @1
      unreachable
    end
    global.get 0
  )
  (func (;3;) (type 4) (param i32) (result i32)
    global.get 1
    i32.eqz
    if ;; label = @1
      unreachable
    end
    local.get 0
    global.get 0
    i32.ge_u
    if ;; label = @1
      unreachable
    end
    local.get 0
    i32.load8_u 1
  )
  (func (;4;) (type 5)
    global.get 1
    i32.eqz
    if ;; label = @1
      unreachable
    end
    i32.const 0
    global.set 1
  )
  (data (;0;) (memory 1) (i32.const 0) "secret!")
)
```

#### After `wasm-opt -O3 --inlining -O3`

**`take_data`:**
```wat
  (func (;0;) (type 0) (result i32)
    (local i32)
    global.get 0
    i32.eqz
    if ;; label = @1
      unreachable
    end
    loop ;; label = @1
      local.get 0
      i32.const 7
      i32.lt_u
      if ;; label = @2
        global.get 0
        i32.eqz
        if ;; label = @3
          unreachable
        end
        local.get 0
        i32.const 7
        i32.ge_u
        if ;; label = @3
          unreachable
        end
        local.get 0
        local.get 0
        i32.load8_u 1
        i32.store8
        local.get 0
        i32.const 1
        i32.add
        local.set 0
        br 1 (;@1;)
      end
    end
    global.get 0
    i32.eqz
    if ;; label = @1
      unreachable
    end
    i32.const 0
    global.set 0
    i32.const 7
  )
```

**`read_after_move`:**
```wat
  (func (;1;) (type 0) (result i32)
    global.get 0
    i32.eqz
    if ;; label = @1
      unreachable
    end
    i32.const 0
    i32.load8_u 1
  )
```

---

### Ownership summary

| Function | Pattern | After optimization |
|---|---|---|
| `count_vowels()` | read-only borrow | accessor inlined → direct `i32.load8_u` from A's memory |
| `uppercase()` | mutable borrow | read+write inlined → direct `i32.load8_u` + `i32.store8` on A's memory |
| `read_back(i)` | mutable borrow | accessor inlined → direct `i32.load8_u` |
| `take_data()` | move | read inlined, `$alive` check + release preserved |
| `read_after_move()` | move | `$alive` check preserved — **traps at runtime** |

**Read-only borrow**: No write accessor exists — security is structural.

**Mutable borrow (call-scoped)**: Synchronous single-threaded Wasm guarantees
mutual exclusion. After optimization, read+write inline to direct memory ops.

**Move / transfer**: After `release()`, A's `$alive` flag traps all subsequent
accesses. The optimizer correctly preserves the check — it cannot prove the
flag's value statically.

## Part 3: Primitives and structs — trivial zero-cost patterns

When the cross-module interface passes primitive values (i32, i64, f32, f64)
or struct fields via individual accessors, values travel on the Wasm stack —
no memory copy, no bounds check needed.

### 7. PRIMITIVE — counter via global, values on the stack

#### Module A
```wat
(module
  ;; Module A (primitive): owns a counter as a mutable global.
  ;; Exports get/increment — B receives values on the stack,
  ;; no memory access needed. Trivially zero-cost after inlining.
  (memory (export "memory") 1)
  (global $counter (mut i32) (i32.const 0))

  (func (export "get_counter") (result i32)
    (global.get $counter))

  (func (export "increment")
    (global.set $counter (i32.add (global.get $counter) (i32.const 1))))
)
```

#### Module B
```wat
(module
  ;; Module B (primitive consumer): calls A's get/increment.
  ;; Values pass on the Wasm stack — no memory copy, no accessor overhead.
  (import "a" "get_counter" (func $get (result i32)))
  (import "a" "increment"   (func $inc))

  (memory (export "b_memory") 1)

  ;; inc_and_get(): increments A's counter, returns the new value.
  ;; After merge + inline: direct global.set + global.get, zero call overhead.
  (func (export "inc_and_get") (result i32)
    (call $inc)
    (call $get))

  ;; inc_n(n): increments A's counter n times, returns final value.
  (func (export "inc_n") (param $n i32) (result i32)
    (local $i i32)
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (call $inc)
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (call $get))
)
```

- merged: `182` bytes

#### After `wasm-merge`
```wat
(module
  (type (;0;) (func (result i32)))
  (type (;1;) (func))
  (type (;2;) (func (param i32) (result i32)))
  (type (;3;) (func (result i32)))
  (type (;4;) (func))
  (memory (;0;) 1)
  (memory (;1;) 1)
  (global (;0;) (mut i32) i32.const 0)
  (export "b_memory" (memory 0))
  (export "inc_and_get" (func 0))
  (export "inc_n" (func 1))
  (export "memory" (memory 1))
  (export "get_counter" (func 2))
  (export "increment" (func 3))
  (func (;0;) (type 0) (result i32)
    call 3
    call 2
  )
  (func (;1;) (type 2) (param i32) (result i32)
    (local i32)
    block ;; label = @1
      loop ;; label = @2
        local.get 1
        local.get 0
        i32.ge_u
        br_if 1 (;@1;)
        call 3
        local.get 1
        i32.const 1
        i32.add
        local.set 1
        br 0 (;@2;)
      end
    end
    call 2
  )
  (func (;2;) (type 3) (result i32)
    global.get 0
  )
  (func (;3;) (type 4)
    global.get 0
    i32.const 1
    i32.add
    global.set 0
  )
)
```

#### After `wasm-opt -O3 --inlining -O3`

**`inc_and_get`:**
```wat
  (func (;0;) (type 0) (result i32)
    global.get 0
    i32.const 1
    i32.add
    global.set 0
    global.get 0
  )
```

**`inc_n`:**
```wat
  (func (;1;) (type 1) (param i32) (result i32)
    (local i32)
    loop ;; label = @1
      local.get 0
      local.get 1
      i32.gt_u
      if ;; label = @2
        global.get 0
        i32.const 1
        i32.add
        global.set 0
        local.get 1
        i32.const 1
        i32.add
        local.set 1
        br 1 (;@1;)
      end
    end
    global.get 0
  )
```

---

### 8. STRUCT — Point {x, y} via field accessors

#### Module A
```wat
(module
  ;; Module A (struct accessor): owns a Point {x: i32, y: i32} in memory.
  ;; Exports field accessors — B reads/writes individual fields,
  ;; never gets a raw pointer to the struct.
  (memory (export "memory") 1)
  (data (i32.const 0) "\03\00\00\00\04\00\00\00")  ;; Point { x: 3, y: 4 }

  (func (export "get_x") (result i32)
    (i32.load (i32.const 0)))

  (func (export "get_y") (result i32)
    (i32.load (i32.const 4)))

  (func (export "set_x") (param $v i32)
    (i32.store (i32.const 0) (local.get $v)))

  (func (export "set_y") (param $v i32)
    (i32.store (i32.const 4) (local.get $v)))
)
```

#### Module B
```wat
(module
  ;; Module B (struct consumer): accesses A's Point through field accessors.
  ;; After merge + inline: direct i32.load/i32.store on A's memory.
  (import "a" "get_x" (func $get_x (result i32)))
  (import "a" "get_y" (func $get_y (result i32)))
  (import "a" "set_x" (func $set_x (param i32)))
  (import "a" "set_y" (func $set_y (param i32)))

  (memory (export "b_memory") 1)

  ;; distance_squared(): returns x*x + y*y without ever seeing raw memory.
  (func (export "distance_squared") (result i32)
    (i32.add
      (i32.mul (call $get_x) (call $get_x))
      (i32.mul (call $get_y) (call $get_y))))

  ;; translate(dx, dy): shifts the point by (dx, dy).
  (func (export "translate") (param $dx i32) (param $dy i32)
    (call $set_x (i32.add (call $get_x) (local.get $dx)))
    (call $set_y (i32.add (call $get_y) (local.get $dy))))
)
```

- merged: `225` bytes

#### After `wasm-merge`
```wat
(module
  (type (;0;) (func (result i32)))
  (type (;1;) (func (param i32)))
  (type (;2;) (func (param i32 i32)))
  (type (;3;) (func (result i32)))
  (type (;4;) (func (param i32)))
  (memory (;0;) 1)
  (memory (;1;) 1)
  (export "b_memory" (memory 0))
  (export "distance_squared" (func 0))
  (export "translate" (func 1))
  (export "memory" (memory 1))
  (export "get_x" (func 2))
  (export "get_y" (func 3))
  (export "set_x" (func 4))
  (export "set_y" (func 5))
  (func (;0;) (type 0) (result i32)
    call 2
    call 2
    i32.mul
    call 3
    call 3
    i32.mul
    i32.add
  )
  (func (;1;) (type 2) (param i32 i32)
    call 2
    local.get 0
    i32.add
    call 4
    call 3
    local.get 1
    i32.add
    call 5
  )
  (func (;2;) (type 3) (result i32)
    i32.const 0
    i32.load 1
  )
  (func (;3;) (type 3) (result i32)
    i32.const 4
    i32.load 1
  )
  (func (;4;) (type 4) (param i32)
    i32.const 0
    local.get 0
    i32.store 1
  )
  (func (;5;) (type 4) (param i32)
    i32.const 4
    local.get 0
    i32.store 1
  )
  (data (;0;) (memory 1) (i32.const 0) "\03\00\00\00\04\00\00\00")
)
```

#### After `wasm-opt -O3 --inlining -O3`

**`distance_squared`:**
```wat
  (func (;0;) (type 0) (result i32)
    (local i32)
    i32.const 0
    i32.load 1
    local.tee 0
    local.get 0
    i32.mul
    i32.const 4
    i32.load 1
    local.tee 0
    local.get 0
    i32.mul
    i32.add
  )
```

**`translate`:**
```wat
  (func (;1;) (type 2) (param i32 i32)
    i32.const 0
    local.get 0
    i32.const 0
    i32.load 1
    i32.add
    i32.store 1
    i32.const 4
    local.get 1
    i32.const 4
    i32.load 1
    i32.add
    i32.store 1
  )
```

---

### Primitives and structs summary

| Function | Pattern | After optimization |
|---|---|---|
| `inc_and_get()` | primitive (global) | direct `global.set` + `global.get` — zero call overhead |
| `inc_n(n)` | primitive (loop) | inlined increment loop — no cross-module call per iteration |
| `distance_squared()` | struct field accessors | direct `i32.load` from A's memory — accessors eliminated |
| `translate(dx, dy)` | struct field accessors | direct `i32.load` + `i32.store` on A's memory |

**Primitives**: values pass on the Wasm stack. No memory involved, no accessor
needed for the return value. After inlining, the call disappears entirely —
`inc_and_get()` becomes a direct global increment and read.

**Structs via field accessors**: each field has its own getter/setter. After
inlining, these become direct `i32.load`/`i32.store` on A's memory at fixed
offsets — identical to accessing a struct in shared memory, but with isolation
preserved.

## Part 4: GC types — type-system enforced isolation

With the Wasm GC proposal, structs and arrays live on the runtime-managed
GC heap — not in linear memory. References pass on the stack, and the type
system enforces field access. No accessor functions or bounds checks needed:
`struct.get`/`struct.set` are already single instructions.

> Note: `wasm-merge` does not yet handle GC types. These examples show the
> pattern — merging GC modules requires GC-aware tooling.

### 9. GC STRUCT — Point {x, y} on the GC heap

#### Module A
```wat
(module
  ;; Module A (GC struct): owns a Point type on the GC heap.
  ;; No linear memory needed — the runtime manages the struct.
  ;; B receives a typed reference and uses struct.get/struct.set directly.
  ;; The type system enforces field access — no accessor functions needed.

  (type $Point (struct (field $x (mut i32)) (field $y (mut i32))))

  ;; Create a new Point on the GC heap.
  (func (export "new_point") (param $x i32) (param $y i32) (result (ref $Point))
    (struct.new $Point (local.get $x) (local.get $y)))

  ;; Read-only access to a Point — returns x + y.
  ;; B could also call struct.get directly if it knows the type.
  (func (export "sum_fields") (param $p (ref $Point)) (result i32)
    (i32.add
      (struct.get $Point $x (local.get $p))
      (struct.get $Point $y (local.get $p))))

  ;; Mutate a field — caller passes the reference, A writes through it.
  (func (export "set_x") (param $p (ref $Point)) (param $v i32)
    (struct.set $Point $x (local.get $p) (local.get $v)))
)
```

#### Module B
```wat
(module
  ;; Module B (GC struct consumer): receives a Point reference from A.
  ;; Uses struct.get/struct.set directly on the reference — no accessor
  ;; wrapper needed. The GC type system enforces field-level access:
  ;; B can only read/write declared fields, cannot forge references,
  ;; and cannot access A's linear memory.
  ;;
  ;; After merge + inline: struct.new and struct.get/set remain as-is —
  ;; they are already single instructions. No call overhead to eliminate.

  (type $Point (struct (field $x (mut i32)) (field $y (mut i32))))

  (import "a" "new_point"  (func $new  (param i32 i32) (result (ref $Point))))
  (import "a" "sum_fields" (func $sum  (param (ref $Point)) (result i32)))
  (import "a" "set_x"      (func $setx (param (ref $Point) i32)))

  ;; distance_squared(): creates a Point(3, 4), computes x*x + y*y.
  ;; The reference lives on the GC heap — no linear memory involved.
  (func (export "distance_squared") (result i32)
    (local $p (ref $Point)) (local $x i32) (local $y i32)
    (local.set $p (call $new (i32.const 3) (i32.const 4)))
    (local.set $x (struct.get $Point $x (local.get $p)))
    (local.set $y (struct.get $Point $y (local.get $p)))
    (i32.add
      (i32.mul (local.get $x) (local.get $x))
      (i32.mul (local.get $y) (local.get $y))))

  ;; mutate_and_sum(): creates a Point(1, 2), changes x to 10, returns sum.
  (func (export "mutate_and_sum") (result i32)
    (local $p (ref $Point))
    (local.set $p (call $new (i32.const 1) (i32.const 2)))
    (call $setx (local.get $p) (i32.const 10))
    (call $sum (local.get $p)))
)
```

### GC types summary

| Pattern | Linear memory accessor | GC struct |
|---|---|---|
| Create | allocate in memory, return offset | `struct.new` — runtime allocates |
| Read field | `get_x()` → `i32.load offset=0` | `struct.get $Point $x` |
| Write field | `set_x(v)` → `i32.store offset=0` | `struct.set $Point $x` |
| Safety | bounds check in accessor | type system — cannot access undeclared fields |
| After inlining | direct `i32.load`/`i32.store` | already a single instruction — nothing to inline |
| Forgery | B cannot forge a pointer (no raw address) | B cannot forge a reference (GC-managed) |

**Linear memory structs** need accessor functions to maintain isolation. After
merge + inline, these become direct loads/stores — zero-cost but requires tooling.

**GC structs** need no accessor functions at all. `struct.get`/`struct.set` are
already single instructions with type-system enforced field access. The isolation
is built into the instruction set. GC types are the natural choice for languages
targeting the GC proposal (Kotlin, Dart, Java, OCaml) while linear memory
accessors serve C/C++/Rust and existing wasi-libc-based toolchains.

## Part 5: Benchmarks — accessor overhead

Measuring 5000000 increment calls on V8:

```
Separate (cross-module call)                11.5 ms
Merged (call preserved)                      3.8 ms (-67%)
Merged + optimized (inlined)                 3.8 ms (-67%)
```

### Benchmark observations

- **Separate modules**: each call crosses the module boundary — the runtime
  cannot inline across modules (V8 compiles at the module level).
- **Merged (call preserved)**: both functions are in the same module but the
  call instruction remains. V8 can now inline at JIT time.
- **Merged + optimized**: `wasm-opt` inlines the call ahead of time. The loop
  body is a direct `global.set` + `global.get` — no call at all.

The progression from separate → merged → optimized shows the full zero-cost
path: module boundary eliminated by merge, call overhead eliminated by inlining.

## Mapping to source languages

| Pattern | Rust | C++ | C |
|---|---|---|---|
| Read-only borrow | `&str` | `string_view` | `const char*` |
| Mutable borrow | `&mut [u8]` | `span<char>` | `char*` (by convention) |
| Move (call-scoped) | `fn(Vec<u8>) -> Vec<u8>` | `vector<uint8_t>&&` + return | annotated macro |
| Move (permanent) | `fn(Vec<u8>)` | `vector<uint8_t>&&` | annotated macro |

## Implications

Component-style shared-nothing linking could become the **default inside a
component** — not just between components. With multi-memory and accessor
functions as the default linking strategy, isolation would be the default
and shared access would be the explicit opt-in, raising the baseline security
of statically linked Wasm without requiring any changes to the Component Model.
