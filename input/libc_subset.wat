(module
  ;; libc_subset — realistic wasi-libc subset with mutable globals.
  ;; This is what clang --target=wasm32 would emit for a minimal libc.
  ;;
  ;; Key properties:
  ;;   - __stack_pointer and __heap_end are mutable globals (per-instance state)
  ;;   - malloc calls sbrk (internal call graph with shared mutable state)
  ;;   - memcpy, memset, strlen are pure leaf functions (no globals)
  ;;
  ;; The vmctx rewriter should:
  ;;   - Rewrite malloc and sbrk to take $vmctx as first parameter
  ;;   - Replace global.get/set with i32.load/store through $vmctx
  ;;   - Leave memcpy, memset, strlen, align_up, free entirely unchanged

  (memory (export "memory") 2)  ;; 128 KiB — room for heap + stack

  ;; ── Per-instance mutable globals ──────────────────────────────────
  ;; These are what need vmctx rewriting.
  (global $__stack_pointer (mut i32) (i32.const 131072))  ;; top of memory
  (global $__heap_end     (mut i32) (i32.const 1024))     ;; heap starts at 1024

  ;; ── sbrk (internal, not exported) ─────────────────────────────────
  ;; Bumps __heap_end by `increment` bytes, returns old value.
  ;; Touches mutable global: __heap_end.
  (func $sbrk (param $increment i32) (result i32)
    (local $old i32)
    (local.set $old (global.get $__heap_end))
    (global.set $__heap_end
      (i32.add (global.get $__heap_end) (local.get $increment)))
    (local.get $old)
  )

  ;; ── align_up (internal, pure) ─────────────────────────────────────
  ;; Rounds n up to the next multiple of align.
  ;; Pure function — no globals, no memory access.
  (func $align_up (param $n i32) (param $align i32) (result i32)
    (i32.and
      (i32.add (local.get $n) (i32.sub (local.get $align) (i32.const 1)))
      (i32.sub (i32.const 0) (local.get $align)))
  )

  ;; ── malloc ────────────────────────────────────────────────────────
  ;; Aligns size to 8 bytes, then calls sbrk.
  ;; State-touching: calls sbrk which accesses __heap_end.
  (func (export "malloc") (param $size i32) (result i32)
    (call $sbrk
      (call $align_up (local.get $size) (i32.const 8)))
  )

  ;; ── free ──────────────────────────────────────────────────────────
  ;; No-op bump allocator — does not touch any state.
  (func (export "free") (param $ptr i32)
    (nop)
  )

  ;; ── memcpy ────────────────────────────────────────────────────────
  ;; Pure leaf function — accesses memory but no globals.
  (func (export "memcpy") (param $dst i32) (param $src i32) (param $n i32) (result i32)
    (local $i i32)
    (local.set $i (i32.const 0))
    (block $break
      (loop $loop
        (br_if $break (i32.ge_u (local.get $i) (local.get $n)))
        (i32.store8
          (i32.add (local.get $dst) (local.get $i))
          (i32.load8_u (i32.add (local.get $src) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.get $dst)
  )

  ;; ── memset ────────────────────────────────────────────────────────
  ;; Pure leaf function — accesses memory but no globals.
  (func (export "memset") (param $dst i32) (param $c i32) (param $n i32) (result i32)
    (local $i i32)
    (local.set $i (i32.const 0))
    (block $break
      (loop $loop
        (br_if $break (i32.ge_u (local.get $i) (local.get $n)))
        (i32.store8
          (i32.add (local.get $dst) (local.get $i))
          (local.get $c))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.get $dst)
  )

  ;; ── strlen ────────────────────────────────────────────────────────
  ;; Pure leaf function — reads memory but no globals.
  (func (export "strlen") (param $s i32) (result i32)
    (local $n i32)
    (local.set $n (i32.const 0))
    (block $break
      (loop $loop
        (br_if $break
          (i32.eqz (i32.load8_u (i32.add (local.get $s) (local.get $n)))))
        (local.set $n (i32.add (local.get $n) (i32.const 1)))
        (br $loop)))
    (local.get $n)
  )
)
