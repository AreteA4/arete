import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = dirname(fileURLToPath(import.meta.url));
const coreRoot = resolve(packageRoot, '../../core');
const temporaryRoot = mkdtempSync(join(tmpdir(), 'arete-adapter-kit-pack-'));
const consumerRoot = join(temporaryRoot, 'consumer');

function npm(args, cwd) {
  return execFileSync(process.platform === 'win32' ? 'npm.cmd' : 'npm', args, {
    cwd,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
}

function assertNoRepositoryRelativeDependencies(packageJson) {
  for (const section of [
    'dependencies',
    'devDependencies',
    'optionalDependencies',
    'peerDependencies',
  ]) {
    for (const [name, specifier] of Object.entries(packageJson[section] ?? {})) {
      assert.equal(
        /^(?:file:|link:|workspace:|\.{1,2}[\\/]|[\\/])/i.test(specifier),
        false,
        `packed ${section}.${name} uses repository-relative specifier ${specifier}`
      );
    }
  }
}

try {
  const packResult = JSON.parse(
    npm(['pack', '--json', '--pack-destination', temporaryRoot], packageRoot)
  )[0];
  const packedFiles = new Set(packResult.files.map(({ path }) => path));
  for (const requiredFile of [
    'LICENSE',
    'README.md',
    'dist/index.cjs',
    'dist/index.d.ts',
    'dist/index.js',
    'package.json',
  ]) {
    assert(packedFiles.has(requiredFile), `packed tarball is missing ${requiredFile}`);
  }

  const tarball = join(temporaryRoot, packResult.filename);
  const packageJson = JSON.parse(readFileSync(join(packageRoot, 'package.json'), 'utf8'));
  mkdirSync(consumerRoot);
  writeFileSync(join(consumerRoot, 'package.json'), JSON.stringify({
    private: true,
    type: 'module',
    dependencies: {
      '@solana/kit': packageJson.devDependencies['@solana/kit'],
      '@usearete/adapter-kit': `file:${tarball}`,
      '@usearete/sdk': `file:${coreRoot}`,
      vite: packageJson.devDependencies.vite,
    },
  }, null, 2));
  npm(['install', '--ignore-scripts', '--no-audit', '--no-fund'], consumerRoot);
  const installedPackageJson = JSON.parse(readFileSync(
    join(consumerRoot, 'node_modules/@usearete/adapter-kit/package.json'),
    'utf8'
  ));
  assertNoRepositoryRelativeDependencies(installedPackageJson);
  cpSync(join(packageRoot, 'smoke/vite'), join(consumerRoot, 'vite'), { recursive: true });

  writeFileSync(join(consumerRoot, 'esm.mjs'), [
    "import assert from 'node:assert/strict';",
    "import * as adapter from '@usearete/adapter-kit';",
    "assert.equal(typeof adapter.createWalletAdapter, 'function');",
    "assert.equal(typeof adapter.KitTransactionExecutionError, 'function');",
  ].join('\n'));
  writeFileSync(join(consumerRoot, 'cjs.cjs'), [
    "const assert = require('node:assert/strict');",
    "const adapter = require('@usearete/adapter-kit');",
    "assert.equal(typeof adapter.createWalletAdapter, 'function');",
    "assert.equal(typeof adapter.KitTransactionExecutionError, 'function');",
  ].join('\n'));

  execFileSync(process.execPath, ['esm.mjs'], { cwd: consumerRoot, stdio: 'inherit' });
  execFileSync(process.execPath, ['cjs.cjs'], { cwd: consumerRoot, stdio: 'inherit' });
  execFileSync(
    join(consumerRoot, 'node_modules', '.bin', process.platform === 'win32' ? 'vite.cmd' : 'vite'),
    ['build', 'vite', '--outDir', join(temporaryRoot, 'vite-dist')],
    { cwd: consumerRoot, stdio: 'inherit' }
  );
  console.log(`Packed ESM/CJS/Vite smoke passed for ${packResult.filename}`);
} finally {
  rmSync(temporaryRoot, { recursive: true, force: true });
}
