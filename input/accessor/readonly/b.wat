(module
  ;; Module B (read-only borrow consumer): reads A's region.
  ;; Cannot modify A's memory — no write accessor available.
  (import "a" "region_len"  (func $len  (result i32)))
  (import "a" "region_read" (func $read (param i32) (result i32)))

  (memory (export "b_memory") 1)

  ;; count_vowels() -> i32: counts vowels in A's region via read accessor.
  (func (export "count_vowels") (result i32)
    (local $i i32) (local $n i32) (local $c i32) (local $end i32)
    (local.set $end (call $len))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $end)))
        (local.set $c (call $read (local.get $i)))
        (if (i32.or
              (i32.or (i32.eq (local.get $c) (i32.const 97))   ;; a
                      (i32.eq (local.get $c) (i32.const 101))) ;; e
              (i32.or (i32.or (i32.eq (local.get $c) (i32.const 105))   ;; i
                              (i32.eq (local.get $c) (i32.const 111)))  ;; o
                      (i32.eq (local.get $c) (i32.const 117))))         ;; u
          (then (local.set $n (i32.add (local.get $n) (i32.const 1)))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.get $n))
)
