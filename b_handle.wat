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
