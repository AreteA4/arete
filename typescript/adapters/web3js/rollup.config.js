import { rmSync } from 'node:fs';
import typescript from '@rollup/plugin-typescript';
import dts from 'rollup-plugin-dts';

rmSync(new URL('./dist', import.meta.url), { recursive: true, force: true });

export default [
  {
    input: 'src/index.ts',
    output: [
      { file: 'dist/index.cjs', format: 'cjs', interop: 'auto', sourcemap: true },
      { file: 'dist/index.js', format: 'esm', sourcemap: true },
    ],
    external: ['@solana/web3.js', '@usearete/sdk', 'bs58'],
    plugins: [
      typescript({ tsconfig: './tsconfig.json', declaration: false }),
    ],
  },
  {
    input: 'src/index.ts',
    output: { file: 'dist/index.d.ts', format: 'es' },
    external: ['@solana/web3.js', '@usearete/sdk', 'bs58'],
    plugins: [dts()],
  },
];
