import resolve from "@rollup/plugin-node-resolve";
import typescript from "@rollup/plugin-typescript";
import dts from "rollup-plugin-dts";

const external = (id) => id === "@noble/hashes" || id.startsWith("@noble/hashes/");

export default [
  {
    input: "src/index.ts",
    external,
    plugins: [
      resolve(),
      typescript({
        tsconfig: "./tsconfig.json",
        declaration: false,
        declarationMap: false,
      }),
    ],
    output: [
      { file: "dist/index.js", format: "esm", sourcemap: true },
      { file: "dist/index.cjs", format: "cjs", sourcemap: true },
    ],
  },
  {
    input: "src/index.ts",
    external,
    plugins: [dts()],
    output: { file: "dist/index.d.ts", format: "es" },
  },
];
