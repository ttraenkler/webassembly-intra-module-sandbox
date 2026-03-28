(module
  ;; Module B (GC struct consumer): receives a Point reference from A.
  ;; Uses struct.get/struct.set directly on the reference — no accessor
  ;; wrapper needed. The GC type system enforces field-level access:
  ;; B can only read/write declared fields, cannot forge references,
  ;; and cannot access A's linear memory.
  ;;
  ;; After merge + inline: struct.new and struct.get/set remain as-is —
  ;; they are already single instructions. No call overhead to eliminate.

  (type $Point (struct (field $x (mut i32)) (field $y (mut i32))))

  (import "a" "new_point"  (func $new  (param i32 i32) (result (ref $Point))))
  (import "a" "sum_fields" (func $sum  (param (ref $Point)) (result i32)))
  (import "a" "set_x"      (func $setx (param (ref $Point) i32)))

  ;; distance_squared(): creates a Point(3, 4), computes x*x + y*y.
  ;; The reference lives on the GC heap — no linear memory involved.
  (func (export "distance_squared") (result i32)
    (local $p (ref $Point)) (local $x i32) (local $y i32)
    (local.set $p (call $new (i32.const 3) (i32.const 4)))
    (local.set $x (struct.get $Point $x (local.get $p)))
    (local.set $y (struct.get $Point $y (local.get $p)))
    (i32.add
      (i32.mul (local.get $x) (local.get $x))
      (i32.mul (local.get $y) (local.get $y))))

  ;; mutate_and_sum(): creates a Point(1, 2), changes x to 10, returns sum.
  (func (export "mutate_and_sum") (result i32)
    (local $p (ref $Point))
    (local.set $p (call $new (i32.const 1) (i32.const 2)))
    (call $setx (local.get $p) (i32.const 10))
    (call $sum (local.get $p)))
)
