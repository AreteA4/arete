import typescript from '@rollup/plugin-typescript';
import resolve from '@rollup/plugin-node-resolve';
import commonjs from '@rollup/plugin-commonjs';
import dts from 'rollup-plugin-dts';

const external = [
  '@usearete/adapter-kit',
  '@usearete/sdk',
  '@usearete/sdk/testing',
  '@wallet-standard/ui-core',
  '@wallet-standard/ui-registry',
  'react',
  'zustand',
  'zustand/middleware',
];

// `src/testing.ts` imports only types from this package's own modules, so the
// testing bundle never carries a second copy of the provider or hooks.
// `src/wallet-standard.ts` imports none of them either; its Wallet Standard
// and adapter-kit imports are optional peers, so the main entry never needs
// them installed.
const entries = [
  { input: 'src/index.ts', name: 'index' },
  { input: 'src/testing.ts', name: 'testing' },
  { input: 'src/wallet-standard.ts', name: 'wallet-standard' },
];

export default entries.flatMap(({ input, name }) => [
  {
    input,
    output: [
      {
        file: `dist/${name}.cjs`,
        format: 'cjs',
        sourcemap: true,
      },
      {
        file: `dist/${name}.esm.js`,
        format: 'esm',
        sourcemap: true,
      },
    ],
    external,
    plugins: [
      resolve({ preferBuiltins: false }),
      commonjs(),
      typescript({
        tsconfig: './tsconfig.json',
        declaration: false,
      }),
    ],
  },
  {
    input,
    output: {
      file: `dist/${name}.d.ts`,
      format: 'es',
    },
    external,
    plugins: [dts()],
  },
]);
