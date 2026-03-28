(module
  ;; Module A (GC struct): owns a Point type on the GC heap.
  ;; No linear memory needed — the runtime manages the struct.
  ;; B receives a typed reference and uses struct.get/struct.set directly.
  ;; The type system enforces field access — no accessor functions needed.

  (type $Point (struct (field $x (mut i32)) (field $y (mut i32))))

  ;; Create a new Point on the GC heap.
  (func (export "new_point") (param $x i32) (param $y i32) (result (ref $Point))
    (struct.new $Point (local.get $x) (local.get $y)))

  ;; Read-only access to a Point — returns x + y.
  ;; B could also call struct.get directly if it knows the type.
  (func (export "sum_fields") (param $p (ref $Point)) (result i32)
    (i32.add
      (struct.get $Point $x (local.get $p))
      (struct.get $Point $y (local.get $p))))

  ;; Mutate a field — caller passes the reference, A writes through it.
  (func (export "set_x") (param $p (ref $Point)) (param $v i32)
    (struct.set $Point $x (local.get $p) (local.get $v)))
)
