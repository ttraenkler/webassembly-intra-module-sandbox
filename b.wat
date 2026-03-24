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
)
