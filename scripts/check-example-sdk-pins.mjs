#!/usr/bin/env node

import { readFileSync } from 'node:fs';

const [manifestPath, ...extensionPaths] = process.argv.slice(2);
if (!manifestPath || extensionPaths.length === 0) {
  console.error('Usage: check-example-sdk-pins.mjs <stack-manifest.json> <extensions.json>...');
  process.exit(1);
}

const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
if (manifest.kind !== 'stack-manifest' || !manifest.artifactHash) {
  throw new Error(`Expected a StackManifest with an artifactHash: ${manifestPath}`);
}

let failed = false;
for (const path of extensionPaths) {
  try {
    const extension = JSON.parse(readFileSync(path, 'utf8'));
    if (extension.inputKind !== 'stack-manifest' || extension.inputHash !== manifest.artifactHash) {
      throw new Error(`expected stack-manifest ${manifest.artifactHash}, found ${extension.inputKind ?? '(missing kind)'} ${extension.inputHash ?? '(missing hash)'}`);
    }
  } catch (error) {
    console.error(`${path}: ${error.message}`);
    failed = true;
  }
}

if (failed) {
  console.error('Review extension compatibility, update each source pin, then rerun scripts/generate-example-sdks.sh.');
  process.exit(1);
}
