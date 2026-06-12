import typescript from '@rollup/plugin-typescript';
import dts from 'rollup-plugin-dts';

export default [
  {
    input: 'src/index.ts',
    output: [
      { file: 'dist/index.js', format: 'cjs', sourcemap: true },
      { file: 'dist/index.esm.js', format: 'esm', sourcemap: true },
    ],
    external: ['@solana/kit', '@usearete/sdk'],
    plugins: [
      typescript({ tsconfig: './tsconfig.json', declaration: false }),
    ],
  },
  {
    input: 'src/index.ts',
    output: { file: 'dist/index.d.ts', format: 'es' },
    external: ['@solana/kit', '@usearete/sdk'],
    plugins: [dts()],
  },
];
