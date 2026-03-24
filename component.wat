;; Component Model version — stretch goal
;;
;; This shows the same two modules expressed as nested core modules
;; within a single component, instantiated and wired together via
;; the component model syntax.
;;
;; NOTE: This is illustrative WAT using the component-model text format.
;; It requires a component-model-aware toolchain (e.g. wasm-tools) to
;; compile and cannot be processed by wat2wasm alone.

(component
  ;; ── Module A (nested core module) ────────────────────────────────
  (core module $A
    (memory (export "memory") 1)
    (data (i32.const 0) "hello")

    (func (export "string_byte") (param $i i32) (result i32)
      (i32.load8_u (local.get $i))
    )
  )

  ;; ── Module B (nested core module) ────────────────────────────────
  (core module $B
    (import "a" "string_byte" (func $string_byte (param i32) (result i32)))

    ;; B's own memory — completely separate from A's
    (memory (export "b_memory") 1)

    (func (export "read_first") (result i32)
      (call $string_byte (i32.const 0))
    )
  )

  ;; ── Instantiation ────────────────────────────────────────────────
  ;; Instantiate A first (no imports needed)
  (core instance $a (instantiate $A))

  ;; Instantiate B, wiring A's export as B's import
  (core instance $b (instantiate $B
    (with "a" (instance $a))
  ))

  ;; ── Lift & export to the component boundary ─────────────────────
  ;; Export read_first as a component-level function
  (func (export "read-first") (result u8)
    (canon lift (core func $b "read_first"))
  )
)
