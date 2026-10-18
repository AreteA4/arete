/**
 * The arete.lock check of `arete-protocol check` and `build`: each installed
 * program module must be the one arete.lock pins, so a protocol never
 * builds against a program SDK that `a4 install` or `a4 update` has since
 * moved.
 *
 * Node only. Reads only the `[[dependency]]` tables of the lock, and only
 * their string fields.
 */

import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

import type { InstalledProgramIdentity } from "../define.js";

/** One `[[dependency]]` table's string fields. */
export type LockedDependencyFields = Record<string, string>;

/** The directory of the nearest `arete.toml` at or above `start`. */
export function findManifestRoot(start: string): string | undefined {
  let directory = resolve(start);
  for (;;) {
    if (existsSync(join(directory, "arete.toml"))) return directory;
    const parent = dirname(directory);
    if (parent === directory) return undefined;
    directory = parent;
  }
}

/** A TOML basic or literal string value, or undefined for anything else. */
function tomlString(value: string): string | undefined {
  const text = value.trim();
  if (text.startsWith("'") && text.endsWith("'") && text.length >= 2) return text.slice(1, -1);
  if (text.startsWith('"') && text.endsWith('"') && text.length >= 2) {
    try {
      return JSON.parse(text) as string;
    } catch {
      return undefined;
    }
  }
  return undefined;
}

/**
 * The string fields of each top-level `[[dependency]]` table of an
 * arete.lock. Nested tables (`[[dependency.programs]]`, ...) and other
 * value types are skipped.
 */
export function parseLockDependencies(text: string): LockedDependencyFields[] {
  const dependencies: LockedDependencyFields[] = [];
  let current: LockedDependencyFields | undefined;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (line.length === 0 || line.startsWith("#")) continue;
    if (line.startsWith("[")) {
      current = line === "[[dependency]]" ? {} : undefined;
      if (current !== undefined) dependencies.push(current);
      continue;
    }
    if (current === undefined) continue;
    const equals = line.indexOf("=");
    if (equals < 0) continue;
    const key = line.slice(0, equals).trim();
    const value = tomlString(line.slice(equals + 1));
    if (value !== undefined && /^[A-Za-z0-9_-]+$/.test(key)) current[key] = value;
  }
  return dependencies;
}

/**
 * Check each installed program against arete.lock (next to the manifest at
 * `manifestRoot`). Returns one finding per mismatch; with no lock it checks
 * nothing and adds a warning.
 */
export function checkProgramLock(
  programs: Readonly<Record<string, InstalledProgramIdentity>>,
  manifestRoot: string | undefined,
  warnings: string[],
): Array<{ message: string }> {
  const lockPath = manifestRoot === undefined ? undefined : join(manifestRoot, "arete.lock");
  if (lockPath === undefined || !existsSync(lockPath)) {
    warnings.push(
      "no arete.lock found, so the installed programs were not checked against it; run `a4 install` in the project",
    );
    return [];
  }
  const locked = parseLockDependencies(readFileSync(lockPath, "utf8")).filter((entry) => entry.kind === "program");
  const problems: Array<{ message: string }> = [];
  for (const [alias, program] of Object.entries(programs)) {
    const install = program.programInstall.alias;
    const name = alias === install ? `'${alias}'` : `'${alias}' (arete.toml '${install}')`;
    const fix = `run \`a4 install\` (or \`a4 update program ${install}\`), then rebuild`;
    const entry = locked.find((candidate) => candidate.alias === install);
    if (entry === undefined) {
      problems.push({ message: `program ${name} is not in arete.lock; ${fix}` });
      continue;
    }
    if (entry.program_spec_hash !== program.programSpecHash) {
      problems.push({
        message: `program ${name} was generated from ProgramSpec ${program.programSpecHash}, but arete.lock pins ${entry.program_spec_hash ?? "none"}; ${fix}`,
      });
      continue;
    }
    const release = program.programInstall.programReleaseHash;
    if (entry.program_release_hash !== release) {
      problems.push({
        message: `program ${name} was generated from release ${release ?? "none"}, but arete.lock pins ${entry.program_release_hash ?? "none"}; ${fix}`,
      });
    }
  }
  return problems;
}
