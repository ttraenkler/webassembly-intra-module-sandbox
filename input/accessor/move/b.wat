(module
  ;; Module B (move consumer): takes ownership of A's region.
  ;; Copies data into B's memory via accessor, then calls release.
  ;; After release, A's accessors trap — use-after-move is caught at runtime.
  (import "a" "region_len"  (func $len     (result i32)))
  (import "a" "region_read" (func $read    (param i32) (result i32)))
  (import "a" "release"     (func $release))

  (memory (export "b_memory") 1)

  ;; take_data() -> i32: copies A's region into B's memory, releases A.
  ;; Returns the length of the copied data.
  (func (export "take_data") (result i32)
    (local $i i32) (local $n i32)
    (local.set $n (call $len))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (i32.store8 (local.get $i) (call $read (local.get $i)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (call $release)
    (local.get $n))

  ;; read_after_move() -> i32: attempts to read from A after transfer — traps.
  (func (export "read_after_move") (result i32)
    (call $read (i32.const 0)))
)
