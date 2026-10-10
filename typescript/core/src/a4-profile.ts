/**
 * The key from the active `a4` CLI login, for servers and local scripts.
 *
 * The `a4` CLI stores the keys it logs in with in a TOML credentials file
 * (`~/.arete/credentials.toml`, or `ARETE_CREDENTIALS_PATH`), keyed by profile
 * and API URL. This module mirrors the CLI's lookup (the Rust SDK's
 * `credentials` module is the reference):
 *
 * - Profile: `.arete/auth.toml` in the working directory (may pin only the
 *   `agent` profile), then `ARETE_PROFILE`, then the single profile holding a
 *   key for the API URL. More than one candidate means no key.
 * - Only the key stored for the default Arete API is used, because that is
 *   where the SDK's default token endpoint sends it. Keys the CLI stored for
 *   another API URL are never sent anywhere else.
 * - Any problem (no file, unreadable, malformed, ambiguous) means no key.
 *
 * Browser safety: nothing here is imported statically from Node. Node builtins
 * are reached at call time through `process.getBuiltinModule` (Node 20.16+,
 * 22.3+, Bun), Deno through its own permission-checked APIs, so bundlers see no
 * `fs` import. Callers must not call this in a browser; `resolveAuthConfig`
 * returns before reaching it there.
 */

/** API URL whose key the SDK may use: its default token endpoint's origin. */
export const DEFAULT_A4_API_URL = 'https://api.arete.run';
const PROFILE_ENV = 'ARETE_PROFILE';
const CREDENTIALS_PATH_ENV = 'ARETE_CREDENTIALS_PATH';
const AGENT_PROFILE = 'agent';
const HUMAN_PROFILE = 'human';

/** Outside world the lookup reads. Tests pass their own. */
export interface A4ProfileHost {
  readEnv(name: string): string | undefined;
  /** File contents, `null` when the file does not exist; throws otherwise. */
  readTextFile(path: string): string | null;
  /** Working directory, if known. */
  cwd(): string | undefined;
  /** Home directory, if known. */
  homeDir(): string | undefined;
  joinPath(...parts: string[]): string;
}

// ── Minimal TOML reader ─────────────────────────────────────────────────────
//
// Covers what the CLI writes (`[a.b."c"]` tables and `key = "string"`
// pairs) plus one-line scalars. Anything else (arrays, inline tables,
// multi-line strings) makes the whole document unreadable, which means
// "no key" rather than a guess.

/** A one-line non-string value (number, boolean, date); its text is unused. */
class TomlScalar {
  constructor(readonly text: string) {}
}
type TomlTable = { [key: string]: TomlValue };
type TomlValue = string | TomlScalar | TomlTable;

class TomlUnsupported extends Error {}

function isTable(value: TomlValue | undefined): value is TomlTable {
  return typeof value === 'object' && !(value instanceof TomlScalar);
}

function readBasicString(line: string, start: number): [string, number] {
  let out = '';
  let index = start + 1;
  while (index < line.length) {
    const char = line[index]!;
    if (char === '"') return [out, index + 1];
    if (char === '\\') {
      const next = line[index + 1];
      const simple: Record<string, string> = {
        b: '\b', t: '\t', n: '\n', f: '\f', r: '\r', '"': '"', '\\': '\\',
      };
      if (next !== undefined && simple[next] !== undefined) {
        out += simple[next];
        index += 2;
        continue;
      }
      if (next === 'u' || next === 'U') {
        const length = next === 'u' ? 4 : 8;
        const hex = line.slice(index + 2, index + 2 + length);
        if (!/^[0-9a-fA-F]+$/.test(hex) || hex.length !== length) throw new TomlUnsupported();
        out += String.fromCodePoint(parseInt(hex, 16));
        index += 2 + length;
        continue;
      }
      throw new TomlUnsupported();
    }
    out += char;
    index += 1;
  }
  throw new TomlUnsupported();
}

function readLiteralString(line: string, start: number): [string, number] {
  const end = line.indexOf("'", start + 1);
  if (end < 0) throw new TomlUnsupported();
  return [line.slice(start + 1, end), end + 1];
}

function skipSpaces(line: string, index: number): number {
  while (index < line.length && (line[index] === ' ' || line[index] === '\t')) index += 1;
  return index;
}

/** Read a dotted key (`a."b".c`) ending at `terminator`. */
function readKey(line: string, start: number, terminator: string): [string[], number] {
  const parts: string[] = [];
  let index = skipSpaces(line, start);
  for (;;) {
    let part: string;
    if (line[index] === '"') {
      [part, index] = readBasicString(line, index);
    } else if (line[index] === "'") {
      [part, index] = readLiteralString(line, index);
    } else {
      const match = /^[A-Za-z0-9_-]+/.exec(line.slice(index));
      if (!match) throw new TomlUnsupported();
      part = match[0];
      index += part.length;
    }
    parts.push(part);
    index = skipSpaces(line, index);
    if (line[index] === '.') {
      index = skipSpaces(line, index + 1);
      continue;
    }
    if (line.startsWith(terminator, index)) return [parts, index + terminator.length];
    throw new TomlUnsupported();
  }
}

function assertRestIsComment(line: string, index: number): void {
  const rest = line.slice(index).trim();
  if (rest !== '' && !rest.startsWith('#')) throw new TomlUnsupported();
}

function tableAt(root: TomlTable, path: string[]): TomlTable {
  let table = root;
  for (const part of path) {
    const existing = table[part];
    if (existing === undefined) {
      const created: TomlTable = Object.create(null) as TomlTable;
      table[part] = created;
      table = created;
    } else if (isTable(existing)) {
      table = existing;
    } else {
      throw new TomlUnsupported();
    }
  }
  return table;
}

/** @internal Parse the subset of TOML described above; throws when unsupported. */
export function parseSimpleToml(content: string): TomlTable {
  const root: TomlTable = Object.create(null) as TomlTable;
  let current = root;
  for (const rawLine of content.replace(/^﻿/, '').split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line === '' || line.startsWith('#')) continue;
    if (line.startsWith('[[')) throw new TomlUnsupported();
    if (line.startsWith('[')) {
      const [path, end] = readKey(line, 1, ']');
      assertRestIsComment(line, end);
      current = tableAt(root, path);
      continue;
    }
    const [path, afterEquals] = readKey(line, 0, '=');
    const valueStart = skipSpaces(line, afterEquals);
    let value: TomlValue;
    let end: number;
    if (line.startsWith('"""', valueStart) || line.startsWith("'''", valueStart)) {
      throw new TomlUnsupported();
    } else if (line[valueStart] === '"') {
      [value, end] = readBasicString(line, valueStart);
    } else if (line[valueStart] === "'") {
      [value, end] = readLiteralString(line, valueStart);
    } else {
      const match = /^[A-Za-z0-9_:.+-]+(?:[ T][0-9:.+Z-]+)?/.exec(line.slice(valueStart));
      if (!match) throw new TomlUnsupported();
      value = new TomlScalar(match[0]);
      end = valueStart + match[0].length;
    }
    assertRestIsComment(line, end);
    const key = path[path.length - 1]!;
    const table = tableAt(current, path.slice(0, -1));
    if (Object.prototype.hasOwnProperty.call(table, key)) throw new TomlUnsupported();
    table[key] = value;
  }
  return root;
}

// ── Lookup (mirror of the Rust `lookup_credentials`) ─────────────────────────

/** Mirror of the CLI's API URL normalisation for credential lookup. */
export function normalizeA4ApiUrl(url: string): string {
  const trimmed = url.trim().replace(/\/+$/, '');
  const schemeAt = trimmed.indexOf('://');
  const scheme = schemeAt >= 0 ? trimmed.slice(0, schemeAt).toLowerCase() : '';
  const rest = schemeAt >= 0 ? trimmed.slice(schemeAt + 3) : trimmed;
  const authorityEnd = rest.search(/[/?#]/);
  const authority = authorityEnd >= 0 ? rest.slice(0, authorityEnd) : rest;
  const tail = authorityEnd >= 0 ? rest.slice(authorityEnd) : '';
  const hostStart = authority.lastIndexOf('@') + 1;
  const hostPort = authority.slice(hostStart);
  let host: string;
  let port: string;
  if (hostPort.startsWith('[')) {
    const close = hostPort.indexOf(']');
    host = close >= 0 ? hostPort.slice(0, close + 1) : hostPort;
    port = close >= 0 ? hostPort.slice(close + 1) : '';
  } else {
    const colon = hostPort.lastIndexOf(':');
    host = colon >= 0 ? hostPort.slice(0, colon) : hostPort;
    port = colon >= 0 ? hostPort.slice(colon) : '';
  }
  host = host.replace(/\.+$/, '').toLowerCase();
  return `${scheme ? `${scheme}://` : ''}${authority.slice(0, hostStart)}${host}${port}${tail}`;
}

function stringMap(value: TomlValue | undefined): Map<string, string> | undefined {
  if (value === undefined) return undefined;
  if (!isTable(value)) throw new TomlUnsupported();
  const out = new Map<string, string>();
  for (const [key, entry] of Object.entries(value)) {
    if (typeof entry !== 'string') throw new TomlUnsupported();
    out.set(key, entry);
  }
  return out;
}

function findUrlKey(keys: Map<string, string> | undefined, apiUrl: string): string | undefined {
  if (!keys) return undefined;
  const wanted = normalizeA4ApiUrl(apiUrl);
  const exact = (keys.get(apiUrl) ?? keys.get(wanted))?.trim();
  if (exact) return exact;
  return [...keys.entries()]
    .filter(([url]) => normalizeA4ApiUrl(url) === wanted)
    .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
    .map(([, key]) => key.trim())
    .find((key) => key !== '');
}

function isValidProfileName(profile: string): boolean {
  return profile.length > 0 && profile.length <= 64 && /^[A-Za-z0-9_-]+$/.test(profile);
}

function keyFitsProfile(profile: string, key: string): boolean {
  const agentKey = key.trim().startsWith('a4_ak_');
  if (profile === AGENT_PROFILE) return agentKey;
  if (profile === HUMAN_PROFILE) return !agentKey;
  return true;
}

/** @internal Result of a credentials lookup. */
export type A4CredentialLookup =
  | { kind: 'key'; key: string }
  | { kind: 'none' }
  | { kind: 'ambiguous' }
  | { kind: 'invalid' };

/** @internal Mirror of the Rust `lookup_credentials`. */
export function lookupA4Credentials(
  content: string,
  apiUrl: string,
  requestedProfile: string | undefined
): A4CredentialLookup {
  let profiles: [string, Map<string, string> | undefined][];
  let legacyKeys: Map<string, string> | undefined;
  let legacyApiKey: string | undefined;
  try {
    const parsed = parseSimpleToml(content);
    const profilesValue = parsed['profiles'];
    if (profilesValue !== undefined && !isTable(profilesValue)) return { kind: 'invalid' };
    profiles = Object.entries(profilesValue ?? {}).map(([name, profile]) => {
      if (!isTable(profile)) throw new TomlUnsupported();
      return [name, stringMap(profile['keys'])];
    });
    legacyKeys = stringMap(parsed['keys']);
    const apiKey = parsed['api_key'];
    if (apiKey !== undefined && typeof apiKey !== 'string') return { kind: 'invalid' };
    legacyApiKey = apiKey?.trim() || undefined;
  } catch {
    return { kind: 'invalid' };
  }
  const legacy = findUrlKey(legacyKeys, apiUrl) ?? legacyApiKey;

  if (requestedProfile !== undefined) {
    if (!isValidProfileName(requestedProfile)) return { kind: 'invalid' };
    const entry = profiles.find(([name]) => name === requestedProfile);
    const key = findUrlKey(entry?.[1], apiUrl);
    if (key !== undefined) {
      return keyFitsProfile(requestedProfile, key) ? { kind: 'key', key } : { kind: 'invalid' };
    }
    if (legacy !== undefined) {
      const compatible = requestedProfile === AGENT_PROFILE
        ? legacy.startsWith('a4_ak_')
        : requestedProfile === HUMAN_PROFILE && !legacy.startsWith('a4_ak_');
      if (compatible) return { kind: 'key', key: legacy };
    }
    return { kind: 'none' };
  }

  const matches = profiles
    .map(([name, keys]) => [name, findUrlKey(keys, apiUrl)] as const)
    .filter((entry): entry is readonly [string, string] => entry[1] !== undefined);
  if (matches.length > 1) return { kind: 'ambiguous' };
  if (matches.length === 1) {
    const [name, key] = matches[0]!;
    return keyFitsProfile(name, key) ? { kind: 'key', key } : { kind: 'invalid' };
  }
  return legacy !== undefined ? { kind: 'key', key: legacy } : { kind: 'none' };
}

/** Selected profile: `undefined` for none, `null` when selection is invalid. */
function selectProfile(host: A4ProfileHost): string | undefined | null {
  const cwd = host.cwd();
  if (cwd !== undefined) {
    let project: string | null;
    try {
      project = host.readTextFile(host.joinPath(cwd, '.arete', 'auth.toml'));
    } catch {
      return null;
    }
    if (project !== null) {
      try {
        const profile = parseSimpleToml(project)['default_profile'];
        return profile === AGENT_PROFILE ? AGENT_PROFILE : null;
      } catch {
        return null;
      }
    }
  }
  const profile = host.readEnv(PROFILE_ENV)?.trim();
  if (profile === undefined) return undefined;
  return isValidProfileName(profile) ? profile : null;
}

/**
 * @internal Key of the active `a4` login for the default Arete API, or
 * `undefined`. Never throws. The caller checks the key class.
 */
export function readA4ProfileKey(
  host: A4ProfileHost | undefined = systemProfileHost()
): { key?: string; ambiguous?: boolean } {
  if (!host) return {};
  try {
    const profile = selectProfile(host);
    if (profile === null) return {};
    const override = host.readEnv(CREDENTIALS_PATH_ENV);
    let path: string | undefined;
    if (override) {
      path = override;
    } else {
      const home = host.homeDir();
      path = home ? host.joinPath(home, '.arete', 'credentials.toml') : undefined;
    }
    if (!path) return {};
    const content = host.readTextFile(path);
    if (content === null) return {};
    const lookup = lookupA4Credentials(content, DEFAULT_A4_API_URL, profile);
    if (lookup.kind === 'key') return { key: lookup.key };
    if (lookup.kind === 'ambiguous') return { ambiguous: true };
    return {};
  } catch {
    return {};
  }
}

// ── Runtime access ──────────────────────────────────────────────────────────

interface NodeFs {
  readFileSync(path: string, encoding: 'utf8'): string;
}
interface NodeOs {
  homedir(): string;
}
interface NodePath {
  join(...parts: string[]): string;
}
interface NodeProcess {
  cwd?: () => string;
  env?: Record<string, string | undefined>;
  getBuiltinModule?: (id: string) => unknown;
}
interface DenoRuntime {
  env?: { get?: (name: string) => string | undefined };
  permissions?: {
    querySync?: (descriptor: { name: string; variable?: string; path?: string }) => { state?: string };
  };
  readTextFileSync?: (path: string) => string;
  cwd?: () => string;
  build?: { os?: string };
  errors?: { NotFound?: new (...args: never[]) => Error };
}

function denoHost(deno: DenoRuntime): A4ProfileHost {
  const granted = (descriptor: { name: string; variable?: string; path?: string }) => {
    try {
      return deno.permissions?.querySync?.(descriptor)?.state === 'granted';
    } catch {
      return false;
    }
  };
  const readEnv = (name: string) =>
    granted({ name: 'env', variable: name }) ? deno.env?.get?.(name) : undefined;
  const separator = deno.build?.os === 'windows' ? '\\' : '/';
  return {
    readEnv,
    readTextFile(path) {
      // Without --allow-read Deno would prompt or throw; ask first.
      if (!granted({ name: 'read', path }) || !deno.readTextFileSync) return null;
      try {
        return deno.readTextFileSync(path);
      } catch (error) {
        const notFound = deno.errors?.NotFound;
        if (notFound && error instanceof notFound) return null;
        throw error;
      }
    },
    cwd() {
      if (!granted({ name: 'read' })) return undefined;
      try {
        return deno.cwd?.();
      } catch {
        return undefined;
      }
    },
    homeDir: () => readEnv('HOME') ?? readEnv('USERPROFILE'),
    joinPath: (...parts) => parts.join(separator),
  };
}

function nodeHost(process: NodeProcess): A4ProfileHost | undefined {
  const load = process.getBuiltinModule;
  if (typeof load !== 'function') return undefined;
  const fs = load.call(process, 'fs') as NodeFs | undefined;
  const os = load.call(process, 'os') as NodeOs | undefined;
  const path = load.call(process, 'path') as NodePath | undefined;
  if (!fs || !os || !path) return undefined;
  return {
    readEnv: (name) => process.env?.[name],
    readTextFile(file) {
      try {
        return fs.readFileSync(file, 'utf8');
      } catch (error) {
        if ((error as { code?: string }).code === 'ENOENT') return null;
        throw error;
      }
    },
    cwd: () => process.cwd?.(),
    homeDir: () => os.homedir(),
    joinPath: (...parts) => path.join(...parts),
  };
}

/** Host for the current server runtime, or `undefined` where files are unreachable. */
export function systemProfileHost(): A4ProfileHost | undefined {
  try {
    const scope = globalThis as { Deno?: DenoRuntime; process?: NodeProcess };
    if (scope.Deno) return denoHost(scope.Deno);
    if (scope.process) return nodeHost(scope.process);
  } catch {
    // Fall through: no profile support in this runtime.
  }
  return undefined;
}
