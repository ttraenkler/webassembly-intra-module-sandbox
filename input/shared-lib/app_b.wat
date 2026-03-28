(module
  ;; App B — instance 1. Imports parameterized wrappers from the library.
  (import "lib" "malloc__inst1" (func $malloc (param i32) (result i32)))
  (memory (export "b_memory") 1)
  (func (export "run_b") (param $size i32) (result i32)
    (call $malloc (local.get $size)))
)
