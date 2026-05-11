// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

/**
 * Copies the compiled .dylib/.so/.dll to a .node file for Node.js to load.
 * Usage: node scripts/copy-node.js [debug|release]
 */
const { copyFileSync, existsSync } = require('fs');
const { join, basename } = require('path');
const { execFileSync } = require('child_process');

const profile = process.argv[2] || 'debug';
const targetDir = join(__dirname, '..', '..', 'target', profile);
const outDir = join(__dirname, '..');

const { platform, arch } = process;

// Determine source library name
let libName;
if (platform === 'win32') {
  libName = 'hyperdb_api_node.dll';
} else if (platform === 'darwin') {
  libName = 'libhyperdb_api_node.dylib';
} else {
  libName = 'libhyperdb_api_node.so';
}

const src = join(targetDir, libName);
const platformTag = `${platform === 'darwin' ? 'darwin' : platform === 'win32' ? 'win32' : 'linux'}-${arch === 'arm64' ? 'arm64' : 'x64'}`;
const dest = join(outDir, `hyperdb-api-node.${platformTag}.node`);

if (!existsSync(src)) {
  console.error(`ERROR: Build artifact not found: ${src}`);
  console.error(`Did 'cargo build -p hyperdb-api-node' succeed?`);
  process.exit(1);
}

copyFileSync(src, dest);
console.log(`Copied ${src} -> ${dest}`);

// On macOS, the compiled dylib embeds an absolute install_name pointing into
// target/. When Node.js dlopen()s the copied .node file, the dynamic linker
// sees that self-referencing path and kills the process (SIGKILL). Rewrite
// the install_name to a relative @loader_path reference so it loads correctly.
if (platform === 'darwin') {
  const newId = `@loader_path/${basename(dest)}`;
  execFileSync('install_name_tool', ['-id', newId, dest]);
  console.log(`Fixed macOS install_name -> ${newId}`);
}
