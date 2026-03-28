(module
  ;; Module B (struct consumer): accesses A's Point through field accessors.
  ;; After merge + inline: direct i32.load/i32.store on A's memory.
  (import "a" "get_x" (func $get_x (result i32)))
  (import "a" "get_y" (func $get_y (result i32)))
  (import "a" "set_x" (func $set_x (param i32)))
  (import "a" "set_y" (func $set_y (param i32)))

  (memory (export "b_memory") 1)

  ;; distance_squared(): returns x*x + y*y without ever seeing raw memory.
  (func (export "distance_squared") (result i32)
    (i32.add
      (i32.mul (call $get_x) (call $get_x))
      (i32.mul (call $get_y) (call $get_y))))

  ;; translate(dx, dy): shifts the point by (dx, dy).
  (func (export "translate") (param $dx i32) (param $dy i32)
    (call $set_x (i32.add (call $get_x) (local.get $dx)))
    (call $set_y (i32.add (call $get_y) (local.get $dy))))
)
