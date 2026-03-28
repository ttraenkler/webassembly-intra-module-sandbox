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
