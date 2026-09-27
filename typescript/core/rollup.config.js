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

// Define all SSR submodules
const ssrModules = [
  'index',
  'handlers',
  'nextjs-app',
  'vite',
  'tanstack-start',
];

export default [
  // Main bundle
  {
    ...baseConfig,
    input: 'src/index.ts',
    output: [
      {
        file: 'dist/index.cjs',
        format: 'cjs',
        sourcemap: true,
      },
      {
        file: 'dist/index.esm.js',
        format: 'esm',
        sourcemap: true,
      },
    ],
  },
  // Type declarations - main
  {
    ...dtsConfig,
    input: 'src/index.ts',
    output: {
      file: 'dist/index.d.ts',
      format: 'es',
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
