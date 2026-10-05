#!/usr/bin/env node
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { createRequire } = require('node:module');
const root = path.resolve(__dirname, '..');
const sdkRequire = createRequire(path.join(root, 'typescript/core/package.json'));
const ts = sdkRequire('typescript');
const filename = path.join(root, 'target/managed-solana-generated/generated.ts');
const options = {
  noEmit: true, strict: true, skipLibCheck: true, target: ts.ScriptTarget.ES2022,
  module: ts.ModuleKind.ESNext, moduleResolution: ts.ModuleResolutionKind.Bundler,
  baseUrl: root, paths: {
    '@usearete/sdk': [path.join(root, 'typescript/core/dist/index.d.ts')],
    zod: [path.join(root, 'typescript/core/node_modules/zod/index.d.ts')],
  },
};
const legacyFilename = path.join(path.dirname(filename), 'legacy.ts');
const legacyStackFilename = path.join(path.dirname(filename), 'legacy-stack.ts');
const program = ts.createProgram([filename, legacyFilename, legacyStackFilename], options);
const diagnostics = ts.getPreEmitDiagnostics(program);
assert.equal(diagnostics.length, 0, ts.formatDiagnosticsWithColorAndContext(diagnostics, {
  getCanonicalFileName: name => name, getCurrentDirectory: () => root, getNewLine: () => '\n',
}));
const source = ts.transpileModule(fs.readFileSync(filename, 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;
const exported = {};
vm.runInNewContext(source, {
  exports: exported, module: { exports: exported },
  require: name => name === '@usearete/sdk' ? sdkRequire('./dist/index.cjs') : sdkRequire(name),
}, { filename, timeout: 10000 });
const fixture = JSON.parse(fs.readFileSync(path.join(root, 'tests/fixtures/managed-solana-v1/dynamic-tick-account.json'), 'utf8'));
const value = exported.TickFixtureSchema.parse(fixture.expected);
assert.equal(value.tick.Initialized.liquidityGross, 1n << 100n);
assert.equal(value.tick.Initialized.liquidityNet, -(1n << 80n));
assert.equal(value.tick.Initialized.rewardGrowthsOutside.length, 3);
assert.equal(exported.TickFixtureSchema.safeParse({ tick: 'Initialized' }).success, false);
assert.equal(exported.TickFixtureSchema.safeParse({ tick: 'Uninitialized' }).success, true);
const payloads = fixture.enumPayloadCases.map(test => exported.PayloadAccountFixtureSchema.parse({ payload: test.expected }).payload);
assert.equal(payloads[0], 'Empty');
assert.equal(payloads[1].Named.exactAmount, (1n << 53n) + 1n);
assert.deepEqual(Array.from(payloads[2].Tuple), [(1n << 53n) + 1n, -((1n << 53n) + 1n)]);
assert.equal(payloads[3].Large.length, 40);
assert.equal(payloads[3].Large[39], (1n << 53n) + 1n);
console.log('Generated TypeScript account models compile and preserve exact dynamic enum payloads.');

const legacy = {};
vm.runInNewContext(ts.transpileModule(fs.readFileSync(legacyFilename, 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText, { exports: legacy, module: { exports: legacy }, require: sdkRequire }, { filename: legacyFilename, timeout: 10000 });
const legacyTick = legacy.DynamicTickSchema.parse(fixture.expected.tick);
assert.equal(legacyTick.Initialized.liquidityGross, 1n << 100n);
assert.equal(legacyTick.Initialized.liquidityNet, -(1n << 80n));
assert.equal(legacy.DynamicTickSchema.safeParse('Initialized').success, false);
console.log('Legacy raw-IDL TypeScript enums compile with their payload-only supporting types.');
