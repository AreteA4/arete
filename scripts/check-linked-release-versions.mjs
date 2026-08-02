#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

function packageVersion(config, packagePath, root) {
  if (config["release-type"] === "node") {
    const manifestPath = path.join(root, packagePath, "package.json");
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
    if (typeof manifest.version !== "string") {
      throw new Error(`${manifestPath} does not declare a package version`);
    }
    return manifest.version;
  }

  if (config["release-type"] === "rust") {
    const manifestPath = path.join(root, packagePath, "Cargo.toml");
    const lines = fs.readFileSync(manifestPath, "utf8").split("\n");
    let inPackage = false;

    for (const line of lines) {
      const section = line.trim().match(/^\[([^[]+)]$/);
      if (section) {
        inPackage = section[1] === "package";
        continue;
      }
      if (inPackage) {
        const version = line.match(/^\s*version\s*=\s*"([^"]+)"/);
        if (version) return version[1];
      }
    }

    throw new Error(`${manifestPath} does not declare [package].version`);
  }

  return undefined;
}

export function validateLinkedReleaseVersions(
  config,
  manifest,
  packageVersions = {},
) {
  const errors = [];
  const summaries = [];
  const packagesByComponent = new Map();

  for (const [packagePath, packageConfig] of Object.entries(
    config.packages ?? {},
  )) {
    const component = packageConfig.component;
    if (!component) continue;

    if (packagesByComponent.has(component)) {
      errors.push(
        `component ${component} is configured for both ${packagesByComponent.get(component).path} and ${packagePath}`,
      );
      continue;
    }
    packagesByComponent.set(component, {
      path: packagePath,
    });
  }

  const groups = (config.plugins ?? []).filter(
    (plugin) => typeof plugin === "object" && plugin.type === "linked-versions",
  );

  for (const group of groups) {
    const versions = new Map();
    const seenComponents = new Set();

    for (const component of group.components ?? []) {
      if (seenComponents.has(component)) {
        errors.push(
          `linked group ${group.groupName} lists ${component} more than once`,
        );
        continue;
      }
      seenComponents.add(component);

      const packageEntry = packagesByComponent.get(component);
      if (!packageEntry) {
        errors.push(
          `linked group ${group.groupName} references unconfigured component ${component}`,
        );
        continue;
      }

      const manifestVersion = manifest[packageEntry.path];
      if (typeof manifestVersion !== "string") {
        errors.push(
          `linked component ${component} is missing manifest version for ${packageEntry.path}`,
        );
        continue;
      }

      versions.set(component, manifestVersion);
      const sourceVersion = packageVersions[packageEntry.path];
      if (sourceVersion !== undefined && sourceVersion !== manifestVersion) {
        errors.push(
          `${component} source version ${sourceVersion} does not match manifest version ${manifestVersion}`,
        );
      }
    }

    const distinctVersions = new Set(versions.values());
    if (distinctVersions.size > 1) {
      const details = [...versions.entries()]
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([component, version]) => `${component}=${version}`)
        .join(", ");
      errors.push(
        `linked group ${group.groupName} has divergent versions: ${details}`,
      );
    } else if (distinctVersions.size === 1) {
      summaries.push(
        `${group.groupName}: ${versions.size} components at ${[...distinctVersions][0]}`,
      );
    }
  }

  return { errors, summaries };
}

function main() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const config = JSON.parse(
    fs.readFileSync(path.join(root, "release-please-config.json"), "utf8"),
  );
  const manifest = JSON.parse(
    fs.readFileSync(path.join(root, ".release-please-manifest.json"), "utf8"),
  );
  const versions = {};

  for (const [packagePath, packageConfig] of Object.entries(
    config.packages ?? {},
  )) {
    const version = packageVersion(packageConfig, packagePath, root);
    if (version !== undefined) versions[packagePath] = version;
  }

  const result = validateLinkedReleaseVersions(config, manifest, versions);
  if (result.errors.length > 0) {
    console.error("Linked release version validation failed:");
    for (const error of result.errors) console.error(`- ${error}`);
    console.error(
      "When a workspace dependency release reaches only part of a linked group, add a release-worthy commit on main that advances arete/.release-please-trigger.",
    );
    process.exit(1);
  }

  for (const summary of result.summaries) {
    console.log(`Linked release versions are synchronized: ${summary}`);
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  main();
}
