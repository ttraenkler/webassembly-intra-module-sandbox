(module
  ;; Module A (move/transfer): owns memory with "secret!".
  ;; After release(), all accessors trap — ownership has been transferred.
  (memory (export "memory") 1)
  (data (i32.const 0) "secret!")

  (global $len   i32 (i32.const 7))
  (global $alive (mut i32) (i32.const 1))  ;; 1 = owned, 0 = moved

  (func (export "region_len") (result i32)
    (if (i32.eqz (global.get $alive)) (then (unreachable)))
    (global.get $len))

  ;; Read accessor — traps after ownership transfer.
  (func (export "region_read") (param $i i32) (result i32)
    (if (i32.eqz (global.get $alive)) (then (unreachable)))
    (if (i32.ge_u (local.get $i) (global.get $len)) (then (unreachable)))
    (i32.load8_u (local.get $i)))

  ;; Release: permanently transfers ownership. All accessors trap after this.
  (func (export "release")
    (if (i32.eqz (global.get $alive)) (then (unreachable)))
    (global.set $alive (i32.const 0)))
)
