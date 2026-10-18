import path from 'node:path';
import { fileURLToPath } from 'node:url';
import typescript from '@rollup/plugin-typescript';
import resolve from '@rollup/plugin-node-resolve';
import dts from 'rollup-plugin-dts';

const externalPackages = ['buffer', 'express', 'next/server', 'pako'];

function isExternal(id) {
  return externalPackages.some(pkg => id === pkg || id.startsWith(`${pkg}/`));
}

const baseConfig = {
  plugins: [
    resolve(),
    typescript({
      tsconfig: './tsconfig.json',
      declaration: false,
      declarationMap: false,
      declarationDir: undefined,
    }),
  ],
  external: isExternal,
};

const dtsConfig = {
  external: isExternal,
  plugins: [dts()],
};

// `@usearete/sdk/testing` imports the SDK only through `src/index.ts`, which
// the published bundle loads from `@usearete/sdk` itself: one copy of every
// class (errors, stores) whichever entry an application imports first.
const SDK_ENTRY = fileURLToPath(new URL('./src/index', import.meta.url));

function sdkEntryAsPackage() {
  return {
    name: 'sdk-entry-as-package',
    resolveId(source, importer) {
      if (!importer || !source.startsWith('.')) return null;
      const resolved = path.resolve(path.dirname(importer), source).replace(/\.(ts|js)$/, '');
      return resolved === SDK_ENTRY ? { id: '@usearete/sdk', external: true } : null;
    },
  };
}

// The root entry and `@usearete/sdk/program`, the pure subset that generated
// program modules import (no client, network, Buffer or Node built-ins).
const SHARED_ENTRIES = {
  index: 'src/index.ts',
  program: 'src/program.ts',
};

// Define all SSR submodules
const ssrModules = [
  'index',
  'handlers',
  'nextjs-app',
  'vite',
  'tanstack-start',
];

export default [
  // Main bundle and `@usearete/sdk/program`, built together so the modules
  // they share (handlers, operation metadata, error classes) land in one
  // chunk: a program built through either entry is the same to the other.
  {
    ...baseConfig,
    input: SHARED_ENTRIES,
    output: [
      {
        dir: 'dist',
        format: 'cjs',
        sourcemap: true,
        entryFileNames: '[name].cjs',
        chunkFileNames: 'chunks/[name]-[hash].cjs',
      },
      {
        dir: 'dist',
        format: 'esm',
        sourcemap: true,
        entryFileNames: '[name].esm.js',
        chunkFileNames: 'chunks/[name]-[hash].esm.js',
      },
    ],
  },
  // Type declarations - main and program, sharing declarations the same way
  // (one `unique symbol` per extension key, one declaration per class).
  {
    ...dtsConfig,
    input: SHARED_ENTRIES,
    output: {
      dir: 'dist',
      format: 'es',
      entryFileNames: '[name].d.ts',
      chunkFileNames: 'chunks/[name]-[hash].d.ts',
    },
  },
  // Testing helpers (`@usearete/sdk/testing`)
  {
    ...baseConfig,
    plugins: [sdkEntryAsPackage(), ...baseConfig.plugins],
    input: 'src/testing/index.ts',
    output: [
      {
        file: 'dist/testing.cjs',
        format: 'cjs',
        sourcemap: true,
      },
      {
        file: 'dist/testing.esm.js',
        format: 'esm',
        sourcemap: true,
      },
    ],
  },
  {
    ...dtsConfig,
    plugins: [sdkEntryAsPackage(), ...dtsConfig.plugins],
    input: 'src/testing/index.ts',
    output: {
      file: 'dist/testing.d.ts',
      format: 'es',
    },
  },
  // SSR modules
  ...ssrModules.flatMap(name => [
    {
      ...baseConfig,
      input: `src/ssr/${name}.ts`,
      output: [
        {
          file: `dist/ssr/${name}.cjs`,
          format: 'cjs',
          sourcemap: true,
        },
        {
          file: `dist/ssr/${name}.esm.js`,
          format: 'esm',
          sourcemap: true,
        },
      ],
    },
    {
      ...dtsConfig,
      input: `src/ssr/${name}.ts`,
      output: {
        file: `dist/ssr/${name}.d.ts`,
        format: 'es',
      },
    },
  ]),
];
