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
