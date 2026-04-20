/**
 * Expressive Code plugin to replace {{VERSION}} placeholders in code blocks.
 *
 * Reads the version from .release-please-manifest.json (source of truth for all packages).
 * All packages are kept in sync, so we use the "arete" version as the canonical one.
 *
 * Usage in code blocks:
 *   arete = "{{VERSION}}"
 *   npm install @usearete/react@{{VERSION}}
 */

import { definePlugin } from "@expressive-code/core";
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));

// Read version from release-please manifest (source of truth)
function getVersion() {
  const manifestPath = resolve(__dirname, "../../../.release-please-manifest.json");
  try {
    const manifest = JSON.parse(readFileSync(manifestPath, "utf-8"));
    // Use the main arete package version (all packages are synchronized)
    return manifest["arete"] || manifest["packages/arete"] || "0.0.0";
  } catch (error) {
    console.warn("Could not read .release-please-manifest.json:", error.message);
    return "0.0.0";
  }
}

export function ecVersionPlugin() {
  const version = getVersion();

  return definePlugin({
    name: "arete-version",
    hooks: {
      preprocessCode: ({ codeBlock }) => {
        // Replace {{VERSION}} in each line of the code block
        // The codeBlock.code is a getter, so we need to modify lines directly
        codeBlock.getLines().forEach((line) => {
          if (line.text.includes("{{VERSION}}")) {
            line.editText(0, line.text.length, line.text.replaceAll("{{VERSION}}", version));
          }
        });
      },
    },
  });
}

export default ecVersionPlugin;
