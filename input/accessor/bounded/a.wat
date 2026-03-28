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
