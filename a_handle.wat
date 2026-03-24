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
