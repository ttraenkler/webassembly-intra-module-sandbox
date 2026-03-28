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
