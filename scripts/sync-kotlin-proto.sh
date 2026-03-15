#!/bin/bash
# scripts/sync-kotlin-proto.sh
# Kotlin metadata proto sync helper.
# Downloads upstream proto files and shows a summary for manual comparison.

set -euo pipefail

KOTLIN_BRANCH="master"
BASE="https://raw.githubusercontent.com/JetBrains/kotlin/${KOTLIN_BRANCH}"

echo "=== Fetching upstream proto files ==="
curl -sL "${BASE}/core/metadata/src/metadata.proto" -o /tmp/metadata.proto
curl -sL "${BASE}/core/metadata.jvm/src/jvm_metadata.proto" -o /tmp/jvm_metadata.proto

echo "=== Upstream metadata.proto message/field summary ==="
grep -n "message\|required\|optional.*=\|repeated.*=" /tmp/metadata.proto | head -80
echo "..."

echo ""
echo "=== Upstream jvm_metadata.proto message/field summary ==="
grep -n "message\|extend\|required\|optional.*=\|repeated.*=" /tmp/jvm_metadata.proto | head -40

echo ""
echo "Compare with: proto/kotlin_metadata.proto"
echo "Apply changes manually, then run: cargo build && cargo test"
