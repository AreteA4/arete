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
    "import { createKeyPairSignerFromPrivateKeyBytes } from '@solana/kit';",
    "import * as adapter from '@usearete/adapter-kit';",
    "assert.equal(typeof adapter.createWalletAdapter, 'function');",
    "assert.equal(typeof adapter.KitTransactionExecutionError, 'function');",
    "assert.equal(typeof adapter.createWalletStandardSigner, 'function');",
    '',
    '// A packed consumer can select V1 and read the typed inspection result.',
    'const signer = await createKeyPairSignerFromPrivateKeyBytes(new Uint8Array(32).fill(1));',
    'const transport = {',
    '  getLatestBlockhash: async () => ({',
    "    blockhash: '11111111111111111111111111111111',",
    '    contextSlot: 1n,',
    '    lastValidBlockHeight: 999n,',
    '  }),',
    '  getFeeForMessage: async () => ({ feeLamports: 5000n, contextSlot: 400n }),',
    '  simulateTransaction: async () => ({',
    '    contextSlot: 401n, err: null, logs: [], unitsConsumed: 1200n,',
    '    loadedAccountsDataSize: 4096n,',
    '  }),',
    '};',
    'const wallet = adapter.createWalletAdapter({ transport, signer });',
    "assert.deepEqual(wallet.supportedTransactionVersions, ['legacy', 0, 1]);",
    'const inspection = await wallet.inspectTransaction(',
    "  [{ programId: 'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr', keys: [], "
      + 'data: new Uint8Array([1]) }],',
    '  { transactionVersion: 1, resources: { priorityFeeLamports: 5000n } }',
    ');',
    'assert.equal(inspection.transactionVersion, 1);',
    'assert.equal(inspection.computeUnitsConsumed, 1200);',
    'assert.equal(inspection.loadedAccountsDataSize, 4096);',
    "assert.equal(inspection.resources.computeUnitLimit, String(adapter.MAX_COMPUTE_UNIT_LIMIT));",
    'assert.equal(',
    '  inspection.resources.loadedAccountsDataSizeLimit,',
    '  String(adapter.MAX_LOADED_ACCOUNTS_DATA_SIZE)',
    ');',
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
