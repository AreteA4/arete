import type { ToolCallRecord } from './types.js';

export type ToolCategory =
  | 'shell'
  | 'read'
  | 'edit'
  | 'search'
  | 'web'
  | 'mcp'
  | 'skill'
  | 'planning'
  | 'subagent'
  | 'other';

export interface A4Invocation {
  /** Subcommand path, e.g. `explore catalog`, `install stack`, `doctor`. */
  path: string;
  help: boolean;
  raw: string;
}

export interface ClassifiedCall {
  record: ToolCallRecord;
  category: ToolCategory;
  command?: string;
  a4: A4Invocation[];
  exitCode?: number;
  failed: boolean;
  mcp?: { server: string; tool: string };
  skillRead?: string;
  docsLookup: boolean;
  directApi: boolean;
  programRun: boolean;
  editedPaths: string[];
  outputText: string;
}

const SHELL_TOOLS = new Set(['bash', 'shell', 'exec_command', 'local_shell', 'run_command', 'PowerShell', 'Monitor']);
const READ_TOOLS = new Set(['read', 'view', 'NotebookRead', 'ReadMcpResourceTool', 'ReadMcpResource']);
const EDIT_TOOLS = new Set(['write', 'edit', 'multiedit', 'apply_patch', 'patch', 'fileChange', 'NotebookEdit']);
const SEARCH_TOOLS = new Set(['grep', 'glob', 'list', 'ls', 'codesearch', 'ToolSearch']);
const WEB_TOOLS = new Set(['webSearch', 'WebFetch', 'webfetch', 'websearch', 'web_search', 'fetch']);
const PLANNING_TOOLS = new Set(['TodoWrite', 'todowrite', 'todoread', 'update_plan', 'TaskCreate', 'TaskUpdate', 'TaskList', 'TaskGet']);
const SUBAGENT_TOOLS = new Set(['Agent', 'task', 'Task']);

/** `a4` commands that take subcommands; every other command is a leaf. */
const A4_GROUPS = new Set(['self', 'explore', 'know', 'sdk', 'config', 'auth', 'stack', 'program', 'telemetry', 'idl']);
const INSTALL_KINDS = new Set(['program', 'stack']);
const GLOBAL_FLAGS_WITH_VALUE = new Set(['--profile', '--config', '--api-url', '--color', '-c']);
/** Prefixes that still leave the next word in command position. */
const COMMAND_PREFIXES = new Set(['sudo', 'time', 'exec', 'command', 'env', 'nohup', 'then', 'do', 'else', '!']);

const AGENT_HOST_DOMAINS = /(?:api\.arete\.run|[a-z0-9-]+\.stack\.arete\.run)/i;
const DOCS_DOMAINS = /(?:docs\.arete\.run|arete\.run\/(?:agent|skill)\.md|github\.com\/AreteA4)/i;

export function outputText(output: unknown): string {
  if (output == null) return '';
  if (typeof output === 'string') return output;
  if (Array.isArray(output)) return output.map(outputText).join('\n');
  if (typeof output === 'object') {
    const obj = output as Record<string, unknown>;
    const parts: string[] = [];
    for (const key of ['stdout', 'stderr', 'output', 'aggregated_output', 'aggregatedOutput', 'formatted_output', 'content', 'text', 'result', 'error', 'value']) {
      const value = obj[key];
      if (value != null) parts.push(outputText(value));
    }
    if (parts.length) return parts.join('\n');
    return JSON.stringify(output);
  }
  return String(output);
}

function exitCodeOf(output: unknown, text: string): number | undefined {
  if (output && typeof output === 'object' && !Array.isArray(output)) {
    const obj = output as Record<string, unknown>;
    const metadata = (obj.metadata ?? {}) as Record<string, unknown>;
    for (const value of [obj.exitCode, obj.exit_code, obj.code, metadata.exit, metadata.exitCode]) {
      if (typeof value === 'number') return value;
    }
  }
  const match =
    /(?:^|\n)\s*(?:Exit code|exit code|Process exited with code|exited with code)[:\s]+(-?\d+)/.exec(text);
  return match ? Number(match[1]) : undefined;
}

/** Codex reports shell calls as `/bin/bash -lc "<command>"`; return the inner command. */
function unwrapShell(command: string): string {
  const match = /^\s*(?:\/(?:usr\/)?bin\/)?(?:ba|z)?sh\s+-l?c\s+(['"])([\s\S]*)\1\s*$/.exec(command);
  if (!match) return command;
  return match[1] === '"' ? match[2]!.replace(/\\(["\\$`])/g, '$1') : match[2]!.replace(/'\\''/g, "'");
}

function commandOf(input: unknown): string | undefined {
  const raw = rawCommandOf(input);
  return raw === undefined ? undefined : unwrapShell(raw);
}

function rawCommandOf(input: unknown): string | undefined {
  if (typeof input === 'string') return input;
  if (input && typeof input === 'object') {
    const obj = input as Record<string, unknown>;
    const value = obj.command ?? obj.cmd ?? obj.script;
    if (typeof value === 'string') return value;
    if (Array.isArray(value)) {
      // Codex reports argv such as ["bash", "-lc", "a4 doctor --json"].
      const last = value[value.length - 1];
      return value.length >= 3 && (value[1] === '-lc' || value[1] === '-c') && typeof last === 'string'
        ? last
        : value.join(' ');
    }
  }
  return undefined;
}

function tokenize(segment: string): string[] {
  return segment
    .trim()
    .split(/\s+/)
    .map((t) => t.replace(/^['"(]+|['")]+$/g, ''))
    .filter(Boolean);
}

/** Drop heredoc bodies so file contents written by `cat <<EOF` are not parsed as commands. */
function stripHeredocs(command: string): string {
  return command.replace(/<<-?\s*['"]?(\w+)['"]?[^\n]*\n[\s\S]*?\n\s*\1\s*(?=\n|$)/g, '<<heredoc');
}

/**
 * Split a command line into simple commands and return the argv of each,
 * starting at the command word (after env assignments and `sudo`-style
 * prefixes). Quoted strings that span lines may produce junk segments, but
 * those rarely start with a command we look for.
 */
export function simpleCommands(command: string): string[][] {
  return stripHeredocs(command)
    .split(/&&|\|\||;|\||\n|\$\(|`/)
    .map((segment) => {
      const tokens = tokenize(segment);
      let i = 0;
      while (i < tokens.length && (COMMAND_PREFIXES.has(tokens[i]!) || /^[A-Za-z_][A-Za-z0-9_]*=/.test(tokens[i]!))) i++;
      if (tokens[i] === 'timeout') i += 2;
      if (tokens[i] === 'cd') return [];
      return tokens.slice(i);
    })
    .filter((argv) => argv.length > 0);
}

function a4Start(argv: string[]): number {
  if (/(?:^|\/)a4$/.test(argv[0]!)) return 1;
  if (argv[0] === 'npx' || argv[0] === 'pnpx' || argv[0] === 'bunx') {
    const i = argv.findIndex((t, idx) => idx > 0 && !t.startsWith('-'));
    if (i > 0 && /^@usearete\/a4(?:@[\w.-]+)?$/.test(argv[i]!)) return i + 1;
  }
  return -1;
}

/** Find every `a4` invocation in a shell command line. */
export function parseA4(command: string): A4Invocation[] {
  const found: A4Invocation[] = [];
  for (const argv of simpleCommands(command)) {
    let i = a4Start(argv);
    if (i < 0) continue;
    const words: string[] = [];
    let help = false;
    for (; i < argv.length; i++) {
      const token = argv[i]!;
      if (token === '--help' || token === '-h') {
        help = true;
        continue;
      }
      if (token.startsWith('-')) {
        if (GLOBAL_FLAGS_WITH_VALUE.has(token) && words.length === 0) i++;
        continue;
      }
      if (words.length === 0) {
        if (!/^[a-z][a-z0-9-]*$/.test(token)) break;
        words.push(token);
        if (!A4_GROUPS.has(token) && token !== 'install') break;
        continue;
      }
      if (words[0] === 'install' && !INSTALL_KINDS.has(token)) break;
      if (/^[a-z][a-z0-9-]*$/.test(token)) words.push(token);
      break;
    }
    if (words[0] === 'help' || argv.slice(i).some((t) => t === '--help' || t === '-h')) help = true;
    found.push({ path: words.join(' ') || '(none)', help, raw: argv.join(' ') });
  }
  return found;
}

const HTTP_CLIENTS = new Set(['curl', 'wget', 'http', 'https', 'xh', 'websocat', 'wscat']);

/** Command lines that make an HTTP or WebSocket request (curl, wget, inline fetch). */
function httpRequests(command: string): string[] {
  return simpleCommands(command)
    .filter((argv) => HTTP_CLIENTS.has(argv[0]!) || ((argv[0] === 'node' || argv[0] === 'python3') && /fetch\(|requests\.|urlopen|WebSocket/.test(argv.join(' '))))
    .map((argv) => argv.join(' '));
}

const PROGRAM_RUNNERS = new Set(['node', 'tsx', 'ts-node', 'bun', 'deno', 'python', 'python3', 'cargo']);

/** True when a command runs a program (as opposed to installing or inspecting). */
function runsProgram(command: string): boolean {
  return simpleCommands(command).some((argv) => {
    const [cmd, ...rest] = argv;
    if (!cmd) return false;
    if (PROGRAM_RUNNERS.has(cmd)) {
      if (cmd === 'cargo') return rest[0] === 'run';
      return rest.some((a) => !a.startsWith('-')) && !rest.includes('--version');
    }
    if (cmd === 'npx' || cmd === 'pnpm' || cmd === 'npm' || cmd === 'yarn') {
      const args = rest.filter((a) => !a.startsWith('-'));
      if (cmd === 'npx' || (cmd === 'pnpm' && args[0] === 'exec')) {
        const tool = cmd === 'npx' ? args[0] : args[1];
        return tool === 'tsx' || tool === 'ts-node';
      }
      return args[0] === 'start' || (args[0] === 'run' && ['start', 'dev'].includes(args[1] ?? ''));
    }
    return false;
  });
}

function patchPaths(patch: string): string[] {
  return [...patch.matchAll(/^\*\*\* (?:Update|Add|Delete) File: (.+)$/gm)].map((m) => m[1]!.trim());
}

function pathOf(input: unknown): string | undefined {
  if (input && typeof input === 'object') {
    const obj = input as Record<string, unknown>;
    const value = obj.file_path ?? obj.filePath ?? obj.path ?? obj.notebook_path;
    if (typeof value === 'string') return value;
  }
  return undefined;
}

function mcpName(name: string): { server: string; tool: string } | undefined {
  // Claude Code and Codex: mcp__<server>__<tool>
  let match = /^mcp__(.+?)__(.+)$/.exec(name);
  if (match) return { server: match[1]!, tool: match[2]! };
  // OpenCode: <server>_<tool>, with the server names `a4 init` writes.
  match = /^(arete-docs|arete_docs|arete)[_.](.+)$/.exec(name);
  if (match) return { server: match[1]!.replace('_', '-'), tool: match[2]! };
  return undefined;
}

function categoryOf(name: string): ToolCategory {
  if (SHELL_TOOLS.has(name)) return 'shell';
  if (READ_TOOLS.has(name)) return 'read';
  if (EDIT_TOOLS.has(name)) return 'edit';
  if (SEARCH_TOOLS.has(name)) return 'search';
  if (WEB_TOOLS.has(name)) return 'web';
  if (PLANNING_TOOLS.has(name)) return 'planning';
  if (SUBAGENT_TOOLS.has(name)) return 'subagent';
  if (name === 'Skill' || name === 'skill') return 'skill';
  if (mcpName(name)) return 'mcp';
  return 'other';
}

export function classify(record: ToolCallRecord): ClassifiedCall {
  const category = categoryOf(record.name);
  const text = outputText(record.output);
  const command = category === 'shell' ? commandOf(record.input) : undefined;
  const a4 = command ? parseA4(command) : [];
  const exitCode = category === 'shell' ? exitCodeOf(record.output, text) : undefined;
  // Pipes such as `| tail` hide a4's exit code, so also look for its error banner.
  const a4ErrorOutput = a4.length > 0 && /^(?:Error|error)(?::|\[)/m.test(text);
  const failed = record.isError || (exitCode !== undefined && exitCode !== 0) || a4ErrorOutput;
  const mcp = category === 'mcp' ? mcpName(record.name) : undefined;
  const path = pathOf(record.input);

  let skillRead: string | undefined;
  if (category === 'skill') {
    const input = (record.input ?? {}) as Record<string, unknown>;
    skillRead = String(input.skill ?? input.name ?? input.command ?? 'skill');
  } else if (category === 'read' && path && /\/skills?\//.test(path)) {
    skillRead = path;
  } else if (command && /\bSKILL\.md\b/.test(command) && /\b(cat|head|less|sed|bat)\b/.test(command)) {
    skillRead = command;
  }

  const inputText = JSON.stringify(record.input ?? '');
  const fetches = command ? httpRequests(command) : [];
  const docsLookup =
    mcp?.server === 'arete-docs' ||
    (category === 'web' && /arete/i.test(inputText)) ||
    fetches.some((argv) => DOCS_DOMAINS.test(argv));
  const directApi = fetches.some((argv) => AGENT_HOST_DOMAINS.test(argv));

  const editedPaths =
    category === 'edit'
      ? typeof record.input === 'string'
        ? patchPaths(record.input)
        : path
          ? [path]
          : patchPaths(String((record.input as Record<string, unknown> | undefined)?.input ?? ''))
      : [];

  return {
    record,
    category,
    command,
    a4,
    exitCode,
    failed,
    mcp,
    skillRead,
    docsLookup,
    directApi,
    programRun: command !== undefined && runsProgram(command),
    editedPaths,
    outputText: text,
  };
}
