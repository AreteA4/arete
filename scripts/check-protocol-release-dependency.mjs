#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const directory = resolve(process.argv[2] ?? 'typescript/protocol');
const protocol = JSON.parse(readFileSync(resolve(directory, 'package.json'), 'utf8'));
const hash = JSON.parse(readFileSync(resolve(directory, '../hash/package.json'), 'utf8'));
const version = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.exec(hash.version);
const major = Number(version?.[1]);
const minor = Number(version?.[2]);
if (protocol.name !== '@usearete/protocol' || hash.name !== '@usearete/hash' ||
    !version ||
    (major === 0 && minor < 7) ||
    protocol.dependencies?.['@usearete/hash'] !== `^${hash.version}`) {
  throw new Error('Protocol publishing requires a released hash version >=0.7.0 and a matching ^version dependency. Review the release PR metadata.');
}
