import resolve from "@rollup/plugin-node-resolve";
import typescript from "@rollup/plugin-typescript";
import dts from "rollup-plugin-dts";

const external = (id) =>
  id === "@usearete/hash" ||
  id === "esbuild" ||
  id.startsWith("@noble/") ||
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
  ...library("src/testing.ts", "testing"),
  {
    input: "src/cli.ts",
    external,
    plugins: [
      resolve(),
      typescript({ tsconfig: "./tsconfig.json", declaration: false, declarationMap: false }),
    ],
    output: {
      file: "dist/cli.js",
      format: "esm",
      sourcemap: false,
      banner: "#!/usr/bin/env node",
    },
  },
];
