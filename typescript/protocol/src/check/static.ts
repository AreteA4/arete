/**
 * Static checks on handler source.
 *
 * This is author feedback, not a sandbox. It catches the obvious ways a
 * handler could reach outside `ctx` (network, filesystem, process,
 * environment, dynamic code) so authors learn early what review and the
 * protocol host will refuse. A determined author can evade a text scan;
 * the real boundaries are human review of every release, the review
 * signature the host verifies, and the host running handlers in a child
 * process with no network.
 */

export const NODE_BUILTIN_MODULES = [
  "assert",
  "async_hooks",
  "buffer",
  "child_process",
  "cluster",
  "console",
  "constants",
  "crypto",
  "dgram",
  "diagnostics_channel",
  "dns",
  "domain",
  "events",
  "fs",
  "http",
  "http2",
  "https",
  "inspector",
  "module",
  "net",
  "os",
  "path",
  "perf_hooks",
  "process",
  "punycode",
  "querystring",
  "readline",
  "repl",
  "stream",
  "string_decoder",
  "sys",
  "timers",
  "tls",
  "trace_events",
  "tty",
  "url",
  "util",
  "v8",
  "vm",
  "wasi",
  "worker_threads",
  "zlib",
] as const;

/** True for `node:*` specifiers and bare Node built-in names (`fs`, `fs/promises`, ...). */
export function isNodeBuiltin(specifier: string): boolean {
  if (specifier.startsWith("node:")) return true;
  const root = specifier.split("/")[0]!;
  return (NODE_BUILTIN_MODULES as readonly string[]).includes(root);
}

/** Globals a handler may not touch directly. */
export const FORBIDDEN_GLOBALS = [
  "fetch",
  "XMLHttpRequest",
  "WebSocket",
  "EventSource",
  "eval",
  "require",
  "process",
  "Deno",
  "Bun",
  "importScripts",
] as const;

export type StaticFindingCode =
  | "forbidden-global"
  | "forbidden-import"
  | "dynamic-import"
  | "dynamic-code"
  | "unresolved-import";

export interface StaticFinding {
  code: StaticFindingCode;
  /** The file the finding is in, relative to the project. */
  file: string;
  /** UTF-16 offset into the file. */
  offset: number;
  /** One-based line and column. */
  line: number;
  column: number;
  message: string;
}

const GLOBAL_OBJECTS = new Set(["globalThis", "window", "self", "global"]);

/**
 * Replace comments, string and template literal text and regular
 * expression literals with spaces, keeping offsets, so the scan only sees
 * code. Module specifiers are collected separately, before masking.
 */
function maskNonCode(source: string): string {
  const out = source.split("");
  let i = 0;
  const templateDepth: number[] = [];
  let braceDepth = 0;
  let lastSignificant = "";
  const blank = (from: number, to: number) => {
    for (let k = from; k < to; k += 1) if (out[k] !== "\n") out[k] = " ";
  };
  const readTemplate = (start: number): number => {
    // From just after a backtick or `}` closing an interpolation, to the next `${` or closing backtick.
    let k = start;
    while (k < source.length) {
      const c = source[k];
      if (c === "\\") {
        k += 2;
        continue;
      }
      if (c === "`") {
        blank(start, k);
        return k + 1;
      }
      if (c === "$" && source[k + 1] === "{") {
        blank(start, k);
        templateDepth.push(braceDepth);
        braceDepth += 1;
        return k + 2;
      }
      k += 1;
    }
    blank(start, k);
    return k;
  };
  while (i < source.length) {
    const c = source[i]!;
    const next = source[i + 1];
    if (c === "/" && next === "/") {
      const end = source.indexOf("\n", i);
      const stop = end < 0 ? source.length : end;
      blank(i, stop);
      i = stop;
      continue;
    }
    if (c === "/" && next === "*") {
      const end = source.indexOf("*/", i + 2);
      const stop = end < 0 ? source.length : end + 2;
      blank(i, stop);
      i = stop;
      continue;
    }
    if (c === '"' || c === "'") {
      let k = i + 1;
      while (k < source.length && source[k] !== c && source[k] !== "\n") k += source[k] === "\\" ? 2 : 1;
      blank(i + 1, k);
      i = k + 1;
      lastSignificant = c;
      continue;
    }
    if (c === "`") {
      i = readTemplate(i + 1);
      lastSignificant = "`";
      continue;
    }
    if (c === "/" && (lastSignificant === "" || /[(,=:[!&|?{};+\-*%<>~^]/.test(lastSignificant))) {
      // A regular expression literal.
      let k = i + 1;
      let inClass = false;
      while (k < source.length && source[k] !== "\n") {
        const r = source[k];
        if (r === "\\") {
          k += 2;
          continue;
        }
        if (r === "[") inClass = true;
        else if (r === "]") inClass = false;
        else if (r === "/" && !inClass) break;
        k += 1;
      }
      blank(i + 1, k);
      i = k + 1;
      lastSignificant = "/";
      continue;
    }
    if (c === "{") braceDepth += 1;
    if (c === "}") {
      braceDepth -= 1;
      if (templateDepth.length > 0 && templateDepth[templateDepth.length - 1] === braceDepth) {
        templateDepth.pop();
        i = readTemplate(i + 1);
        lastSignificant = "`";
        continue;
      }
    }
    if (!/\s/.test(c)) lastSignificant = c;
    i += 1;
  }
  return out.join("");
}

function position(source: string, offset: number): { line: number; column: number } {
  let line = 1;
  let column = 1;
  for (let i = 0; i < offset; i += 1) {
    if (source[i] === "\n") {
      line += 1;
      column = 1;
    } else {
      column += 1;
    }
  }
  return { line, column };
}

const IMPORT_PATTERNS = [
  /\bimport\s+(?:type\s+)?[^'"`;]*?\bfrom\s*(['"])([^'"]+)\1/g,
  /\bexport\s+(?:type\s+)?[^'"`;]*?\bfrom\s*(['"])([^'"]+)\1/g,
  /\bimport\s*(['"])([^'"]+)\1/g,
];

/** Module specifiers a file imports statically, with their offsets. */
export function staticImports(source: string): Array<{ specifier: string; offset: number; typeOnly: boolean }> {
  const found: Array<{ specifier: string; offset: number; typeOnly: boolean }> = [];
  const code = maskNonCodeKeepingStrings(source);
  for (const pattern of IMPORT_PATTERNS) {
    pattern.lastIndex = 0;
    for (let match = pattern.exec(code); match !== null; match = pattern.exec(code)) {
      found.push({
        specifier: match[2]!,
        offset: match.index,
        typeOnly: /^(?:import|export)\s+type\s/.test(match[0]),
      });
    }
  }
  return found.sort((a, b) => a.offset - b.offset);
}

/** Mask comments only, so import specifiers stay readable. */
function maskNonCodeKeepingStrings(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " "))
    .replace(/(^|[^:"'`\\])\/\/[^\n]*/g, (m, p: string) => p + " ".repeat(m.length - p.length));
}

/**
 * Scan one handler source file (TypeScript or JavaScript) and return every
 * finding. `file` is only used to label findings.
 */
export function checkHandlerSource(source: string, file = "<source>"): StaticFinding[] {
  const findings: StaticFinding[] = [];
  const add = (code: StaticFindingCode, offset: number, message: string) =>
    findings.push({ code, file, offset, ...position(source, offset), message });

  for (const { specifier, offset, typeOnly } of staticImports(source)) {
    if (!typeOnly && isNodeBuiltin(specifier)) {
      add("forbidden-import", offset, `'${specifier}' is a Node built-in; handlers reach data only through ctx`);
    }
  }

  const code = maskNonCode(source);
  const prevIndex = (from: number): number => {
    let k = from - 1;
    while (k >= 0 && /\s/.test(code[k]!)) k -= 1;
    return k;
  };
  const nextChar = (from: number): string => {
    let k = from;
    while (k < code.length && /\s/.test(code[k]!)) k += 1;
    return code[k] ?? "";
  };
  const wordBefore = (end: number): string => {
    let k = end;
    while (k >= 0 && /[A-Za-z0-9_$]/.test(code[k]!)) k -= 1;
    return code.slice(k + 1, end + 1);
  };
  const identifier = /[A-Za-z_$][A-Za-z0-9_$]*/g;
  for (let match = identifier.exec(code); match !== null; match = identifier.exec(code)) {
    const name = match[0];
    const start = match.index;
    if (start > 0 && /[A-Za-z0-9_$]/.test(code[start - 1]!)) continue;
    const prev = prevIndex(start);
    const prevChar = prev >= 0 ? code[prev]! : "";
    const after = nextChar(start + name.length);
    const memberOf = prevChar === "." && code[prev - 1] !== ".";
    const objectName = memberOf ? wordBefore(prevIndex(prev)) : undefined;
    const viaGlobal = objectName !== undefined && GLOBAL_OBJECTS.has(objectName);
    const isKey = !memberOf && (after === ":" || after === "?") && /[{,;]/.test(prevChar);
    const keyword = /[A-Za-z0-9_$]/.test(prevChar) ? wordBefore(prev) : "";

    if (name === "import" && after === "(" && !memberOf) {
      add("dynamic-import", start, "dynamic import() is not allowed; import modules statically so they are bundled");
      continue;
    }
    if (name === "Function" && (!memberOf || viaGlobal) && (after === "(" || keyword === "new")) {
      add("dynamic-code", start, "the Function constructor evaluates code at runtime and is not allowed");
      continue;
    }
    if ((FORBIDDEN_GLOBALS as readonly string[]).includes(name) && (!memberOf || viaGlobal) && !isKey) {
      if (["const", "let", "var", "function", "class"].includes(keyword)) continue;
      const kind = name === "eval" ? "dynamic-code" : "forbidden-global";
      add(kind, start, `'${name}' is not available to handlers; use ctx for data`);
    }
  }
  return findings.sort((a, b) => a.offset - b.offset);
}
