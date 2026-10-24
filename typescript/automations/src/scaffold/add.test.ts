import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

import { main } from '../cli';
import { add, configuredEntry } from './add';
import { importPath } from './templates';

function project(entry = 'automations/index.ts') {
  const root = mkdtempSync(join(tmpdir(), 'arete-add-'));
  mkdirSync(join(root, entry, '..'), { recursive: true });
  writeFileSync(join(root, entry), 'export default {};\n');
  writeFileSync(join(root, 'package.json'), JSON.stringify({ name: 'tip-jar' }));
  writeFileSync(join(root, 'arete.toml'), `[project]\nname = "tip-jar"\n\n[dev.automations]\nentry = "${entry}" # the definition\n`);
  return root;
}

const read = (root: string, path: string) => readFileSync(join(root, path), 'utf8');

describe('arete-automations add', () => {
  it('writes a Worker over the shared runtime, with the deploy steps', () => {
    const root = project();
    const result = add({ root, target: 'cloudflare' });
    expect(result.written).toEqual(['automations/runtime.ts', 'worker.ts', 'wrangler.jsonc']);
    expect(read(root, 'worker.ts')).toContain("import definition from './automations/index';");
    expect(read(root, 'worker.ts')).toContain("import { runtime } from './automations/runtime';");
    expect(read(root, 'automations/runtime.ts')).toContain("import definition from './index';");
    expect(read(root, 'automations/runtime.ts')).toContain("keypairSigner(JSON.parse(need(env, 'SIGNER_KEYPAIR'))");
    expect(read(root, 'wrangler.jsonc')).toContain('"name": "tip-jar-automations"');
    expect(read(root, 'wrangler.jsonc')).toContain('"crons": ["* * * * *"]');
    expect(result.devDependencies).toEqual(['wrangler', '@types/pg']);
    expect(result.next).toContain('a4 install --locked                     # bundle the published client, not the one a4 dev generated');
    expect(result.next.at(-1)).toBe(
      'a4 protocol webhook add --url https://<worker>.workers.dev/arete/webhook --secret-only | npx wrangler secret put ARETE_WEBHOOK_SECRET',
    );
  });

  it('keeps existing files, and shares runtime.ts between targets', () => {
    const root = project();
    add({ root, target: 'node' });
    writeFileSync(join(root, 'vercel.json'), '{"framework":null}\n');
    const result = add({ root, target: 'vercel', signer: 'privy' });
    expect(result.written).toEqual(['api/arete/[route].ts']);
    expect(result.kept).toEqual(['automations/runtime.ts', 'vercel.json']);
    expect(read(root, 'vercel.json')).toBe('{"framework":null}\n');
    expect(read(root, 'api/arete/[route].ts')).toContain("import definition from '../../automations/index';");
    expect(result.dependencies).toContain('@privy-io/node');
    expect(read(root, 'server.ts')).toContain("await serve(definition, runtime);");
  });

  it('writes the Privy signer on request', () => {
    const root = project('src/automations.ts');
    expect(configuredEntry(root)).toBe('src/automations.ts');
    add({ root, target: 'node', signer: 'privy' });
    const runtime = read(root, 'src/runtime.ts');
    expect(runtime).toContain("import definition from './automations';");
    expect(runtime).toContain("authorizationPrivateKey: need(env, 'PRIVY_AUTHORIZATION_KEY')");
    expect(read(root, 'server.ts')).toContain("import definition from './src/automations';");
  });

  it('refuses a missing definition and an unknown target', () => {
    const root = mkdtempSync(join(tmpdir(), 'arete-add-'));
    expect(() => add({ root, target: 'node' })).toThrow(/automations\/index.ts does not exist/);
    const errors: string[] = [];
    const out = { log: () => {}, error: (text: string) => errors.push(text) };
    expect(main(['add', 'heroku', '--dir', root], out)).toBe(1);
    expect(errors[0]).toContain('cloudflare, vercel, node');
  });

  it('deploys Vercel again once the webhook secret is stored, and keeps env files out of images', () => {
    const vercel = add({ root: project(), target: 'vercel' });
    expect(vercel.next.at(-2)).toMatch(/^a4 protocol webhook add .*npx vercel env add ARETE_WEBHOOK_SECRET production$/);
    expect(vercel.next.at(-1)).toMatch(/^npx vercel deploy --prod /);
    const root = project();
    add({ root, target: 'node' });
    expect(read(root, '.dockerignore')).toContain('.env\n.env.*\n!.env.example\n');
  });

  it('refuses an entry where runtime.ts would be written', () => {
    const root = project('automations/runtime.ts');
    expect(() => add({ root, target: 'node' })).toThrow(/is where runtime.ts is written/);
    expect(() => read(root, 'server.ts')).toThrow();
  });

  it('computes import paths between project files', () => {
    expect(importPath('worker.ts', 'automations/index')).toBe('./automations/index');
    expect(importPath('automations/runtime', 'automations/index')).toBe('./index');
    expect(importPath('api/arete/[route].ts', 'automations/runtime')).toBe('../../automations/runtime');
    expect(importPath('src/runtime', 'lib/automations')).toBe('../lib/automations');
  });
});
