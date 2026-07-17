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
