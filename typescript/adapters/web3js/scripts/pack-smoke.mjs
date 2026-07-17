import { cp, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const coreRoot = resolve(packageRoot, '../../core');
const temporaryRoot = await mkdtemp(join(tmpdir(), 'arete-web3js-pack-'));
const consumerRoot = join(temporaryRoot, 'consumer');

function run(command, args, cwd) {
  execFileSync(command, args, { cwd, stdio: 'inherit' });
}

function pack(directory) {
  const output = execFileSync(
    'npm',
    ['pack', '--json', '--pack-destination', temporaryRoot],
    { cwd: directory, encoding: 'utf8' }
  );
  const metadata = JSON.parse(output);
  return join(temporaryRoot, metadata[0].filename);
}

try {
  const adapterTarball = pack(packageRoot);
  const coreTarball = pack(coreRoot);
  await cp(join(packageRoot, 'smoke'), consumerRoot, { recursive: true });
  await writeFile(
    join(consumerRoot, 'package.json'),
    JSON.stringify({ name: 'adapter-web3js-packed-smoke', private: true, type: 'module' })
  );

  run(
    'npm',
    [
      'install',
      '--ignore-scripts',
      '--no-audit',
      '--no-fund',
      '--no-package-lock',
      adapterTarball,
      coreTarball,
      '@solana/web3.js@^1.95.0',
      'vite@^6.4.3',
    ],
    consumerRoot
  );

  run('node', ['esm.mjs'], consumerRoot);
  run('node', ['cjs.cjs'], consumerRoot);
  run(
    join(consumerRoot, 'node_modules', '.bin', 'vite'),
    ['build', 'vite', '--outDir', join(temporaryRoot, 'vite-dist')],
    consumerRoot
  );

  const assetsRoot = join(temporaryRoot, 'vite-dist', 'assets');
  const browserBundle = (await Promise.all(
    (await readdir(assetsRoot))
      .filter((file) => file.endsWith('.js'))
      .map((file) => readFile(join(assetsRoot, file), 'utf8'))
  )).join('\n');
  if (/externalized for browser compatibility|__vite-browser-external/.test(browserBundle)) {
    throw new Error('Packed adapter Vite output contains a browser-externalized Node module');
  }

  const packedManifest = JSON.parse(
    await readFile(join(consumerRoot, 'node_modules', '@usearete', 'adapter-web3js', 'package.json'), 'utf8')
  );
  if (packedManifest.dependencies?.bs58 !== '^6.0.0') {
    throw new Error('Packed adapter does not declare the tested bs58 dependency');
  }
  if (packedManifest.peerDependencies?.['rpc-websockets'] !== '9.3.4') {
    throw new Error('Packed adapter does not require the CommonJS-compatible rpc-websockets release');
  }
  if (packedManifest.dependencies?.buffer !== undefined) {
    throw new Error('Packed adapter must not depend on the Node buffer polyfill');
  }
  for (const section of ['dependencies', 'optionalDependencies', 'peerDependencies']) {
    for (const [name, range] of Object.entries(packedManifest[section] ?? {})) {
      if (typeof range !== 'string' || /^(?:file|link):/.test(range)) {
        throw new Error(`Packed adapter contains an invalid ${section} entry for ${name}`);
      }
    }
  }
} finally {
  await rm(temporaryRoot, { recursive: true, force: true });
}
