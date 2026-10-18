import { isBuiltin } from 'node:module';
import { fileURLToPath } from 'node:url';

import { build, type Plugin } from 'esbuild';
import { describe, expect, it } from 'vitest';

import * as root from './index';
import * as program from './program';

const ENTRY = fileURLToPath(new URL('./program.ts', import.meta.url));

/** Any Node built-in import fails the bundle, as it would in a sandboxed host. */
const rejectNodeBuiltins: Plugin = {
  name: 'reject-node-builtins',
  setup(pluginBuild) {
    pluginBuild.onResolve({ filter: /.*/ }, (args) => {
      if (args.kind === 'entry-point') return undefined;
      if (!isBuiltin(args.path) && args.path !== 'buffer' && args.path !== 'buffer/') return undefined;
      return { errors: [{ text: `'${args.path}' is a Node built-in` }] };
    });
  },
};

/**
 * Host surfaces a program module must never reach. Strings and comments are
 * stripped first, so only code references count.
 */
const HOST_GLOBALS = ['WebSocket', 'fetch', 'XMLHttpRequest', 'EventSource', 'process', 'require', 'Buffer'];

function stripStringsAndComments(code: string): string {
  return code
    .replace(/\/\*[\s\S]*?\*\//g, ' ')
    .replace(/\/\/[^\n]*/g, ' ')
    .replace(/`(?:\\[\s\S]|[^`\\])*`/g, '``')
    .replace(/"(?:\\.|[^"\\\n])*"/g, '""')
    .replace(/'(?:\\.|[^'\\\n])*'/g, "''");
}

async function bundleProgramEntry() {
  const result = await build({
    entryPoints: [ENTRY],
    bundle: true,
    platform: 'neutral',
    format: 'esm',
    target: 'es2022',
    mainFields: ['module', 'main'],
    write: false,
    metafile: true,
    logLevel: 'silent',
    legalComments: 'none',
    plugins: [rejectNodeBuiltins],
  });
  return { text: result.outputFiles[0]!.text, inputs: Object.keys(result.metafile.inputs) };
}

describe('@usearete/sdk/program', () => {
  it('bundles for a neutral platform without Node built-ins or the client', async () => {
    const { text, inputs } = await bundleProgramEntry();

    const packages = inputs
      .filter((input) => input.includes('node_modules/'))
      .map((input) => input.replace(/^.*node_modules\/((?:@[^/]+\/)?[^/]+).*$/, '$1'));
    expect([...new Set(packages)].sort()).toEqual(['@noble/ed25519']);

    const sources = inputs.filter((input) => !input.includes('node_modules/'));
    for (const forbidden of ['client', 'connection', 'session', 'chain', 'solana-gateway', 'transactions', 'stream', 'subscription']) {
      expect(sources.some((source) => source.endsWith(`src/${forbidden}.ts`)), forbidden).toBe(false);
    }

    const code = stripStringsAndComments(text);
    for (const name of HOST_GLOBALS) {
      expect(code, name).not.toMatch(new RegExp(`(?<![\\w$])${name}(?![\\w$])`));
    }
  });

  it('is re-exported unchanged by the root entry', () => {
    const rootExports = root as unknown as Record<string, unknown>;
    for (const [name, value] of Object.entries(program)) {
      expect(rootExports[name], name).toBe(value);
    }
  });

  it('exports every runtime helper a generated program module imports', () => {
    expect(Object.keys(program).sort()).toEqual([
      'PROGRAM_OPERATION_EXTENSIONS',
      'account',
      'arg',
      'buildInstruction',
      'bytes',
      'createInstructionHandler',
      'createPreparedInstruction',
      'defineProgramExtensions',
      'extendProgram',
      'instructionOperation',
      'literal',
      'pda',
      'programAccountRead',
      'resolveAmountToRaw',
      'toRawAmount',
      'withProgramIdentity',
      'withProgramRead',
    ]);
  });
});
