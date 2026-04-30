import { readFile, writeFile } from "node:fs/promises";

// Postbuild: prepend a top-level markdown heading + description to llms-full.txt
// so it satisfies the AFDocs "valid markdown structure" check. The starlight-llms-txt
// plugin emits the file starting with a <SYSTEM> tag and no H1.
const path = "dist/llms-full.txt";
const original = await readFile(path, "utf8");

const HEADER = `# Arete — Full Documentation

> Arete is a system for programmable real-time data feeds on Solana. Stream any on-chain data to your app via WebSocket. Define data shapes in a Rust DSL, deploy, and consume with typed TypeScript, React, or Rust SDKs.

This document concatenates every page of the Arete documentation. For a smaller index with links, see [llms.txt](https://docs.arete.run/llms.txt). For an abridged version, see [llms-small.txt](https://docs.arete.run/llms-small.txt).

---

`;

if (!original.startsWith("# Arete — Full Documentation")) {
  await writeFile(path, HEADER + original);
  console.log("[fix-llms-full] prepended markdown header to", path);
} else {
  console.log("[fix-llms-full] already prefixed, skipping");
}
