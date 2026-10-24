import resolve from "@rollup/plugin-node-resolve";
import typescript from "@rollup/plugin-typescript";
import dts from "rollup-plugin-dts";

const external = (id) =>
  id.startsWith("@scure/") ||
  id.startsWith("@noble/") ||
  id.startsWith("@usearete/") ||
  id.startsWith("node:");

const library = (input, name) => [
  {
    input,
    external,
    plugins: [
      resolve(),
      typescript({ tsconfig: "./tsconfig.json", declaration: false, declarationMap: false }),
    ],
    output: [
      { file: `dist/${name}.js`, format: "esm", sourcemap: true },
      { file: `dist/${name}.cjs`, format: "cjs", sourcemap: true },
    ],
  },
  {
    input,
    external,
    plugins: [dts()],
    output: { file: `dist/${name}.d.ts`, format: "es" },
  },
];

export default [
  ...library("src/index.ts", "index"),
  ...library("src/keypair.ts", "keypair"),
  ...library("src/testing.ts", "testing"),
  ...library("src/postgres.ts", "postgres"),
  ...library("src/privy.ts", "privy"),
  ...library("src/cloudflare.ts", "cloudflare"),
  ...library("src/vercel.ts", "vercel"),
  ...library("src/node.ts", "node"),
  {
    input: "src/cli-bin.ts",
    external,
    plugins: [
      resolve(),
      typescript({ tsconfig: "./tsconfig.json", declaration: false, declarationMap: false }),
    ],
    output: { file: "dist/cli.js", format: "esm", sourcemap: false, inlineDynamicImports: true, banner: "#!/usr/bin/env node" },
  },
];
