(module
  ;; Parameter-threaded version: $instance_idx is a function parameter.
  ;; 2 memories (one per instance), 4 mutable globals (2 per instance).
  ;; No dispatch global — the parameter IS the dispatch.

  (memory (;0;) 2)  ;; instance 0
  (memory (;1;) 2)  ;; instance 1

  (global $sp_0 (mut i32) (i32.const 131072))
  (global $sp_1 (mut i32) (i32.const 131072))
  (global $he_0 (mut i32) (i32.const 1024))
  (global $he_1 (mut i32) (i32.const 1024))

  ;; ── dispatch wrappers (br_table, parameter-based) ──

  (func $get_heap_end (param $idx i32) (result i32)
    (block $default (block $b1 (block $b0
      (br_table $b0 $b1 $default (local.get $idx)))
      (return (global.get $he_0)))
      (return (global.get $he_1)))
    (unreachable))

  (func $set_heap_end (param $val i32) (param $idx i32)
    (block $default (block $b1 (block $b0
      (br_table $b0 $b1 $default (local.get $idx)))
      (global.set $he_0 (local.get $val)) (return))
      (global.set $he_1 (local.get $val)) (return))
    (unreachable))

  ;; ── library functions (parameter-threaded) ──

  (func $align_up (param $n i32) (param $align i32) (result i32)
    (i32.and
      (i32.add (local.get $n) (i32.sub (local.get $align) (i32.const 1)))
      (i32.sub (i32.const 0) (local.get $align))))

  (func $sbrk (param $inc i32) (param $idx i32) (result i32)
    (local $old i32)
    (local.set $old (call $get_heap_end (local.get $idx)))
    (call $set_heap_end
      (i32.add (call $get_heap_end (local.get $idx)) (local.get $inc))
      (local.get $idx))
    (local.get $old))

  (func $malloc_impl (param $size i32) (param $idx i32) (result i32)
    (call $sbrk
      (call $align_up (local.get $size) (i32.const 8))
      (local.get $idx)))

  ;; ── consumer stubs ──

  (func (export "run_a") (param $size i32) (result i32)
    (call $malloc_impl (local.get $size) (i32.const 0)))

  (func (export "run_b") (param $size i32) (result i32)
    (call $malloc_impl (local.get $size) (i32.const 1)))
)
