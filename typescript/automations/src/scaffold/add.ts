/**
 * `arete-automations add <target>`: write a host's entry and config beside
 * the project's definition, plus `runtime.ts` when the project has none.
 * Existing files are never overwritten; what was kept is reported so it can
 * be merged by hand. Deploying is the host's own CLI's job.
 */

import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

import { requiredEnv, runtimeTs, type SignerKind, type Target, targetTemplate, TARGETS } from './templates';

export interface AddOptions {
  readonly root: string;
  readonly target: Target;
  readonly signer?: SignerKind;
  /** The definition module, relative to the root; defaults to `[dev.automations] entry` in arete.toml, else `automations/index.ts`. */
  readonly entry?: string;
}

export interface AddResult {
  readonly written: string[];
  /** Files that already existed and were left alone. */
  readonly kept: string[];
  readonly dependencies: readonly string[];
  readonly devDependencies: readonly string[];
  /** What to run next, in order. */
  readonly next: string[];
  readonly notes: readonly string[];
}

export function isTarget(value: string | undefined): value is Target {
  return TARGETS.includes(value as Target);
}

/** `entry` under `[dev.automations]` in arete.toml, when set. */
export function configuredEntry(root: string): string | undefined {
  let toml: string;
  try {
    toml = readFileSync(join(root, 'arete.toml'), 'utf8');
  } catch {
    return undefined;
  }
  let inSection = false;
  for (const raw of toml.split('\n')) {
    const line = raw.replace(/#.*$/, '').trim();
    if (line.startsWith('[')) {
      inSection = line === '[dev.automations]';
      continue;
    }
    const match = inSection ? /^entry\s*=\s*"([^"]+)"$/.exec(line) : null;
    if (match) return match[1];
  }
  return undefined;
}

function packageName(root: string): string {
  try {
    const name = (JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')) as { name?: unknown }).name;
    return typeof name === 'string' && name !== '' ? name : 'arete';
  } catch {
    return 'arete';
  }
}

const withoutExtension = (path: string) => path.replace(/\.[cm]?[jt]sx?$/, '');

export function add(options: AddOptions): AddResult {
  const signer = options.signer ?? 'keypair';
  const entryFile = (options.entry ?? configuredEntry(options.root) ?? 'automations/index.ts').replace(/^\.\//, '');
  if (!existsSync(join(options.root, entryFile))) {
    throw new Error(
      `${entryFile} does not exist: point --entry at the module exporting defineAutomations(...) (a4 protocol init writes automations/index.ts)`,
    );
  }
  const entryDir = dirname(entryFile);
  const runtimeFile = entryDir === '.' ? 'runtime.ts' : `${entryDir}/runtime.ts`;
  if (withoutExtension(entryFile) === withoutExtension(runtimeFile)) {
    throw new Error(`${entryFile} is where runtime.ts is written: keep the definition in another file (automations/index.ts)`);
  }
  const input = {
    entry: withoutExtension(entryFile),
    runtime: withoutExtension(runtimeFile),
    packageName: packageName(options.root),
    signer,
  };
  const template = targetTemplate(options.target, input);
  const files = [{ path: runtimeFile, content: runtimeTs(input) }, ...template.files];

  const written: string[] = [];
  const kept: string[] = [];
  for (const file of files) {
    const target = join(options.root, file.path);
    if (existsSync(target)) {
      kept.push(file.path);
      continue;
    }
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, file.content);
    written.push(file.path);
  }

  const install = [
    `npm install ${template.dependencies.join(' ')}`,
    ...(template.devDependencies.length ? [`npm install --save-dev ${template.devDependencies.join(' ')}`] : []),
  ];
  const secrets = requiredEnv(signer).map((name) =>
    name === 'ARETE_WEBHOOK_SECRET'
      ? `a4 protocol webhook add --url ${template.webhookUrl} --secret-only | ${template.secretCommand.replace('{name}', name)}`
      : template.secretCommand.replace('{name}', name),
  );
  const next = [
    ...install,
    'a4 protocol deploy                      # the protocol, if this release is not deployed yet',
    'a4 install --locked                     # bundle the published client, not the one a4 dev generated',
    'DATABASE_URL=... npx arete-automations migrate',
    ...secrets.filter((line) => !line.startsWith('a4 protocol webhook')),
    template.deployCommand,
    // The webhook needs the deployment's address, so its secret is stored after the first deploy.
    ...secrets.filter((line) => line.startsWith('a4 protocol webhook')),
    ...(template.redeployForSecrets ? [`${template.deployCommand}                 # environment changes reach new deployments only`] : []),
  ];
  return {
    written,
    kept,
    dependencies: template.dependencies,
    devDependencies: template.devDependencies,
    next,
    notes: template.notes,
  };
}
