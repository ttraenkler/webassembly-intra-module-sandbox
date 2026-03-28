(module
  ;; Module B (mutable borrow consumer): reads and writes A's region.
  ;; Uppercases "hello world" in-place via accessor functions.
  (import "a" "region_len"   (func $len   (result i32)))
  (import "a" "region_read"  (func $read  (param i32) (result i32)))
  (import "a" "region_write" (func $write (param i32 i32)))

  (memory (export "b_memory") 1)

  ;; uppercase(): converts A's region to uppercase in-place.
  (func (export "uppercase")
    (local $i i32) (local $c i32) (local $end i32)
    (local.set $end (call $len))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $end)))
        (local.set $c (call $read (local.get $i)))
        (if (i32.and
              (i32.ge_u (local.get $c) (i32.const 97))
              (i32.le_u (local.get $c) (i32.const 122)))
          (then
            (call $write (local.get $i) (i32.sub (local.get $c) (i32.const 32)))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop))))

  ;; read_back(i) -> i32: reads byte i from A to verify mutation persisted.
  (func (export "read_back") (param $i i32) (result i32)
    (call $read (local.get $i)))
)
