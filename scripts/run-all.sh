#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

echo "=== 1/3 demo-sandbox-merge ==="
scripts/demo-sandbox-merge.sh
echo ""

echo "=== 2/3 demo-shared-memory-accessor ==="
scripts/demo-shared-memory-accessor.sh
echo ""

echo "=== 3/3 benchmark-shared-library ==="
scripts/benchmark-shared-library.sh
node scripts/bench-format.mjs > docs/BENCHMARK.md
echo "Wrote docs/BENCHMARK.md"
echo ""

echo "=== All done ==="
