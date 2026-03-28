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
