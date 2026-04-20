/**
 * Remark plugin to replace {{VERSION}} placeholders with the current package version.
 *
 * Reads the version from .release-please-manifest.json (source of truth for all packages).
 * All packages are kept in sync, so we use the "arete" version as the canonical one.
 *
 * Usage in MDX files:
 *   arete = "{{VERSION}}"
 *   npm install @usearete/react@{{VERSION}}
 */

import { visit } from "unist-util-visit";
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

export function remarkVersion() {
  const version = getVersion();

  return (tree) => {
    // Replace in text nodes
    visit(tree, "text", (node) => {
      if (node.value.includes("{{VERSION}}")) {
        node.value = node.value.replaceAll("{{VERSION}}", version);
      }
    });

    // Replace in code blocks (fenced code)
    visit(tree, "code", (node) => {
      if (node.value.includes("{{VERSION}}")) {
        node.value = node.value.replaceAll("{{VERSION}}", version);
      }
    });

    // Replace in inline code
    visit(tree, "inlineCode", (node) => {
      if (node.value.includes("{{VERSION}}")) {
        node.value = node.value.replaceAll("{{VERSION}}", version);
      }
    });
  };
}

export default remarkVersion;
