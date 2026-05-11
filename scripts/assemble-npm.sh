#!/usr/bin/env bash
# Copyright (c) 2026, Salesforce, Inc. All rights reserved.
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Assembles npm packages for local sharing or testing.
# Copies the release-built hyperdb-mcp binary and hyperd into the npm
# platform directories, then runs `npm pack` to produce .tgz files.
#
# Prerequisites:
#   - `make build-release` (or `cargo build --release -p hyperdb-mcp -p hyperdb-api-node`)
#   - `make download-hyperd` (hyperd available at .hyperd/current/)
#
# Usage:
#   ./scripts/assemble-npm.sh
#
# Output:
#   hyperdb-mcp/npm/*.tgz           — MCP server packages
#   hyperdb-api-node/npm/*.tgz      — Node.js binding packages (if built)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HYPERD_DIR="${ROOT}/.hyperd/current"

# Detect platform
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)  PLATFORM="darwin-arm64";  EXE=""; HYPERD_BIN="hyperd" ;;
  Darwin-x86_64) PLATFORM="darwin-x64";    EXE=""; HYPERD_BIN="hyperd" ;;
  Linux-x86_64)  PLATFORM="linux-x64-gnu"; EXE=""; HYPERD_BIN="hyperd" ;;
  MINGW*|MSYS*)  PLATFORM="win32-x64-msvc"; EXE=".exe"; HYPERD_BIN="hyperd.exe" ;;
  *)
    echo "ERROR: Unsupported platform: $(uname -s)-$(uname -m)" >&2
    exit 1
    ;;
esac

echo "Platform: ${PLATFORM}"
echo "hyperd dir: ${HYPERD_DIR}"

# Verify prerequisites
if [[ ! -f "${ROOT}/target/release/hyperdb-mcp${EXE}" ]]; then
  echo "ERROR: target/release/hyperdb-mcp${EXE} not found. Run 'make build-release' first." >&2
  exit 1
fi

if [[ ! -f "${HYPERD_DIR}/${HYPERD_BIN}" ]]; then
  echo "ERROR: ${HYPERD_DIR}/${HYPERD_BIN} not found. Run 'make download-hyperd' first." >&2
  exit 1
fi

# --- hyperdb-mcp ---
MCP_DEST="${ROOT}/hyperdb-mcp/npm/${PLATFORM}"
echo ""
echo "=== Assembling hyperdb-mcp (${PLATFORM}) ==="
cp "${ROOT}/target/release/hyperdb-mcp${EXE}" "${MCP_DEST}/"
cp "${HYPERD_DIR}/${HYPERD_BIN}" "${MCP_DEST}/"
# Copy shared libraries hyperd needs
find "${HYPERD_DIR}" -maxdepth 1 \( -name "*.so*" -o -name "*.dylib" -o -name "*.dll" \) -exec cp {} "${MCP_DEST}/" \;
chmod +x "${MCP_DEST}/hyperdb-mcp${EXE}" "${MCP_DEST}/${HYPERD_BIN}" 2>/dev/null || true
echo "Contents:"
ls -lh "${MCP_DEST}/"

# Pack platform package
echo ""
echo "Packing hyperdb-mcp-${PLATFORM}..."
(cd "${MCP_DEST}" && npm pack --quiet)

# Pack main package
echo "Packing hyperdb-mcp (main)..."
(cd "${ROOT}/hyperdb-mcp/npm" && npm pack --quiet)

# --- hyperdb-api-node ---
NODE_DEST="${ROOT}/hyperdb-api-node/npm/${PLATFORM}"
NODE_BIN="${ROOT}/target/release/libhyperdb_api_node"

# Determine the native addon filename
case "${PLATFORM}" in
  darwin-*) NODE_LIB="${NODE_BIN}.dylib" ;;
  linux-*)  NODE_LIB="${NODE_BIN}.so" ;;
  win32-*)  NODE_LIB="${ROOT}/target/release/hyperdb_api_node.dll" ;;
esac

if [[ -f "${NODE_LIB}" ]]; then
  echo ""
  echo "=== Assembling hyperdb-api-node (${PLATFORM}) ==="
  # Use the copy-node.js script which handles the rename + install_name fix
  node "${ROOT}/hyperdb-api-node/scripts/copy-node.js" release
  # Copy the .node file into the npm platform dir
  NODE_FILE=$(ls "${ROOT}/hyperdb-api-node/hyperdb-api-node.${PLATFORM}.node" 2>/dev/null || true)
  if [[ -n "$NODE_FILE" ]]; then
    cp "$NODE_FILE" "${NODE_DEST}/"
  fi
  # Copy hyperd
  cp "${HYPERD_DIR}/${HYPERD_BIN}" "${NODE_DEST}/"
  find "${HYPERD_DIR}" -maxdepth 1 \( -name "*.so*" -o -name "*.dylib" -o -name "*.dll" \) -exec cp {} "${NODE_DEST}/" \;
  chmod +x "${NODE_DEST}/${HYPERD_BIN}" 2>/dev/null || true
  echo "Contents:"
  ls -lh "${NODE_DEST}/"

  echo ""
  echo "Packing hyperdb-api-node-${PLATFORM}..."
  (cd "${NODE_DEST}" && npm pack --quiet)
else
  echo ""
  echo "Skipping hyperdb-api-node (native addon not built for this platform)."
  echo "Build with: cargo build --release -p hyperdb-api-node"
fi

echo ""
echo "=== Done ==="
echo ""
echo "To install locally:"
echo "  npm install ${MCP_DEST}/hyperdb-mcp-${PLATFORM}-0.1.0.tgz ${ROOT}/hyperdb-mcp/npm/hyperdb-mcp-0.1.0.tgz"
echo ""
echo "Or test directly:"
echo "  node ${ROOT}/hyperdb-mcp/npm/bin.js"
