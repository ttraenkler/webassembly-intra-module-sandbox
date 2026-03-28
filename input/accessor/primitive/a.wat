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
