(module
  ;; Module B (primitive consumer): calls A's get/increment.
  ;; Values pass on the Wasm stack — no memory copy, no accessor overhead.
  (import "a" "get_counter" (func $get (result i32)))
  (import "a" "increment"   (func $inc))

  (memory (export "b_memory") 1)

  ;; inc_and_get(): increments A's counter, returns the new value.
  ;; After merge + inline: direct global.set + global.get, zero call overhead.
  (func (export "inc_and_get") (result i32)
    (call $inc)
    (call $get))

  ;; inc_n(n): increments A's counter n times, returns final value.
  (func (export "inc_n") (param $n i32) (result i32)
    (local $i i32)
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (call $inc)
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (call $get))
)
