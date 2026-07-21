import { rmSync } from 'node:fs';
import typescript from '@rollup/plugin-typescript';
import dts from 'rollup-plugin-dts';

rmSync(new URL('./dist', import.meta.url), { recursive: true, force: true });

const external = [
  '@solana/web3.js',
  '@usearete/sdk',
  'bs58',
  'react',
  '@solana/wallet-adapter-react',
];

export default [
  {
    input: { index: 'src/index.ts', react: 'src/react.ts' },
    output: [
      {
        dir: 'dist',
        format: 'cjs',
        entryFileNames: '[name].cjs',
        chunkFileNames: 'shared-[hash].cjs',
        interop: 'auto',
        sourcemap: true,
      },
      {
        dir: 'dist',
        format: 'esm',
        entryFileNames: '[name].js',
        chunkFileNames: 'shared-[hash].js',
        sourcemap: true,
      },
    ],
    external,
    plugins: [
      typescript({ tsconfig: './tsconfig.json', declaration: false }),
    ],
  },
  {
    input: { index: 'src/index.ts', react: 'src/react.ts' },
    output: { dir: 'dist', format: 'es', entryFileNames: '[name].d.ts' },
    external,
    plugins: [dts()],
  },
];
