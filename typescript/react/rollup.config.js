import typescript from '@rollup/plugin-typescript';
import resolve from '@rollup/plugin-node-resolve';
import commonjs from '@rollup/plugin-commonjs';
import dts from 'rollup-plugin-dts';

const external = [
  '@usearete/sdk',
  '@usearete/sdk/testing',
  'react',
  'zustand',
  'zustand/middleware',
];

// `src/testing.ts` imports only types from this package's own modules, so the
// testing bundle never carries a second copy of the provider or hooks.
const entries = [
  { input: 'src/index.ts', name: 'index' },
  { input: 'src/testing.ts', name: 'testing' },
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
