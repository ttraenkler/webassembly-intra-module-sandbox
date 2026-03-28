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
