import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import type { SandboxShell } from './types.js';

export type FileCategory = 'app' | 'sdk' | 'arete-config' | 'config' | 'other';

export interface WorkspaceEntry {
  path: string;
  lines: number;
  hash: string;
}

export interface WorkspaceFile {
  path: string;
  lines: number;
  category: FileCategory;
}

export interface WorkspaceDiff {
  created: WorkspaceFile[];
  modified: WorkspaceFile[];
  deleted: string[];
  /** Lines in created + modified files, per category. */
  lines: Record<FileCategory, number>;
}

/** Excluded from snapshots, diffs and downloads. */
export const WORKSPACE_EXCLUDES = ['node_modules', '.git', 'dist', 'build', 'target', '.cache', '.next', '.npm', '.pnpm-store', '__pycache__', '.venv'];

/** Paths `a4 install` generates; agents should regenerate rather than edit them. */
const SDK_PATTERNS = [/^src\/arete\//, /(^|\/)generated\//, /^arete\/sdk\//];

/** Files `a4 init` and the agent hosts manage. */
const ARETE_CONFIG = [
  /^arete\.(toml|lock)$/,
  /^skills-lock\.json$/,
  /^(AGENTS|CLAUDE)\.md$/,
  /^\.mcp\.json$/,
  /^opencode\.jsonc?$/,
  /^\.(arete|claude|codex|agents|opencode|gemini)\//,
];

const CONFIG_FILES = /(^|\/)(package(-lock)?\.json|pnpm-lock\.yaml|yarn\.lock|bun\.lockb?|tsconfig[\w.-]*\.json|\.gitignore|\.env[\w.-]*|Cargo\.(toml|lock)|pyproject\.toml|requirements\.txt)$/;

export function classifyPath(path: string): FileCategory {
  if (SDK_PATTERNS.some((p) => p.test(path))) return 'sdk';
  if (ARETE_CONFIG.some((p) => p.test(path))) return 'arete-config';
  if (CONFIG_FILES.test(path)) return 'config';
  if (/\.(ts|tsx|js|jsx|mjs|cjs|py|rs|css|html|svelte|vue)$/.test(path)) return 'app';
  return 'other';
}

/** One round-trip listing of every workspace file with its line count and hash. */
export async function snapshotWorkspace(shell: SandboxShell): Promise<WorkspaceEntry[]> {
  const prune = WORKSPACE_EXCLUDES.map((d) => `-name ${d} -prune -o`).join(' ');
  const result = await shell.run(
    `find . ${prune} -type f -print0 2>/dev/null | sort -z | while IFS= read -r -d '' f; do ` +
      `printf '%s\\t%s\\t%s\\n' "$f" "$(wc -l < "$f" 2>/dev/null || echo 0)" "$(md5sum < "$f" | cut -c1-16)"; done`,
    { timeoutSeconds: 120 },
  );
  return result.stdout
    .split('\n')
    .map((line) => line.split('\t'))
    .filter((parts) => parts.length === 3)
    .map(([path, lines, hash]) => ({
      path: path!.replace(/^\.\//, ''),
      lines: Number.parseInt(lines!, 10) || 0,
      hash: hash!,
    }));
}

export function diffWorkspace(before: WorkspaceEntry[], after: WorkspaceEntry[]): WorkspaceDiff {
  const beforeByPath = new Map(before.map((e) => [e.path, e]));
  const afterPaths = new Set(after.map((e) => e.path));
  const diff: WorkspaceDiff = {
    created: [],
    modified: [],
    deleted: before.filter((e) => !afterPaths.has(e.path)).map((e) => e.path),
    lines: { app: 0, sdk: 0, 'arete-config': 0, config: 0, other: 0 },
  };
  for (const entry of after) {
    const prev = beforeByPath.get(entry.path);
    if (prev && prev.hash === entry.hash) continue;
    const file = { path: entry.path, lines: entry.lines, category: classifyPath(entry.path) };
    (prev ? diff.modified : diff.created).push(file);
    diff.lines[file.category] += file.lines;
  }
  return diff;
}

/**
 * Copy sandbox directories to a local directory via a tarball. Missing
 * sources are skipped. Returns false when nothing could be copied.
 */
export async function downloadDirs(
  shell: SandboxShell,
  baseDir: string,
  relativeDirs: string[],
  localDir: string,
  excludes: string[] = [],
): Promise<boolean> {
  const archive = `/tmp/bench-${Math.random().toString(36).slice(2)}.tgz`;
  const exclude = excludes.map((e) => `--exclude=${e}`).join(' ');
  const list = relativeDirs.map((d) => `'${d}'`).join(' ');
  const pack = await shell.run(
    `cd '${baseDir}' && existing=$(for d in ${list}; do [ -e "$d" ] && printf '%s ' "$d"; done); ` +
      `[ -n "$existing" ] && tar czf ${archive} ${exclude} $existing`,
    { timeoutSeconds: 180 },
  );
  if (pack.exitCode !== 0) return false;
  const bytes = await shell.readBinary(archive);
  await shell.run(`rm -f ${archive}`);
  if (!bytes) return false;
  mkdirSync(localDir, { recursive: true });
  const localArchive = join(localDir, '.download.tgz');
  writeFileSync(localArchive, bytes);
  try {
    execFileSync('tar', ['xzf', localArchive, '-C', localDir]);
  } finally {
    rmSync(localArchive, { force: true });
  }
  return true;
}
