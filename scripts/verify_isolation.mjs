// Verify state isolation: A and B get independent heap state
// despite sharing the same library function bodies.

import { readFileSync } from 'fs';

const wasm = readFileSync('vmctx_optimized.wasm');
const mod = new WebAssembly.Module(wasm);
const instance = new WebAssembly.Instance(mod);

const { run_a, run_b } = instance.exports;

// A allocates 16 bytes (instance 0)
const a1 = run_a(16);
console.log(`run_a(16) = ${a1}`);

// B allocates 16 bytes (instance 1) — should start at B's heap base, not A's
const b1 = run_b(16);
console.log(`run_b(16) = ${b1}`);

// A allocates again — should advance from A's last position, not B's
const a2 = run_a(16);
console.log(`run_a(16) = ${a2}`);

// B allocates again
const b2 = run_b(32);
console.log(`run_b(32) = ${b2}`);

// Verify isolation
// Both start at 1024 (initial __heap_end) — same address is expected since
// they have separate memories and separate global copies.
console.log('');

if (a2 === a1 + 16) {
  console.log(`✓ A's heap advanced by 16 (${a1} → ${a2})`);
} else {
  console.log(`✗ A's heap did not advance correctly (expected ${a1 + 16}, got ${a2})`);
}

if (b2 === b1 + 16) {
  console.log(`✓ B's heap advanced by 16 (${b1} → ${b2})`);
} else {
  console.log(`✗ B's heap did not advance correctly (expected ${b1 + 16}, got ${b2})`);
}

// Key test: B's allocation did not affect A's next allocation
if (a2 === a1 + 16 && b1 === a1) {
  console.log(`✓ State isolated: B's malloc(16) did not affect A's heap pointer`);
  console.log(`  (A: ${a1}→${a2}, B: ${b1}→${b2} — both start at ${a1}, advance independently)`);
} else if (a2 !== a1 + 16) {
  console.log(`✗ State NOT isolated: A's heap was affected by B's allocation`);
}

console.log('');
console.log('Shared function bodies, independent per-instance heap state.');
