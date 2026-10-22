// A stand-in for the module `a4 install protocol token-raise` generates, so
// this example typechecks on its own. It uses the generator's export names
// (TOKEN_RAISE_PROTOCOL, TokenRaiseProtocolTypes): replace this file with a
// re-export of the generated module, which carries the real deployment,
// hashes and types.

import type { ProtocolDefinition, ProtocolTypeMap } from '@usearete/sdk';

export interface TokenRaiseProtocolTypes extends ProtocolTypeMap {
  readonly params: object;
  readonly instanceState: 'open' | 'succeeded' | 'failed' | 'finalized';
  readonly participantState: 'contributed' | 'claimed' | 'refunded';
  readonly instanceMetadata: object;
  readonly participantMetadata: object;
  readonly steps: {
    readonly contribute: { scope: 'participant'; access: 'public'; args: { amount: string }; guardCode: string };
    readonly claim: { scope: 'participant'; access: 'public'; args: Record<string, never>; guardCode: 'raise_not_finalized' };
    readonly refund: { scope: 'participant'; access: 'public'; args: Record<string, never>; guardCode: 'raise_not_failed' };
  };
  readonly events: {
    readonly 'raise.succeeded': { machine: 'instance' };
    readonly 'raise.failed': { machine: 'instance' };
    readonly 'contribution.confirmed': { machine: 'participant' };
  };
}

export const TOKEN_RAISE_PROTOCOL: ProtocolDefinition<TokenRaiseProtocolTypes> = {
  kind: 'protocol',
  name: 'tokenRaise',
  version: '0.0.0',
  package: 'token-raise',
  packageReleaseHash: 'sha256:replace-with-the-generated-module',
  protocolSpecHash: 'sha256:replace-with-the-generated-module',
  protocolReleaseHash: 'sha256:replace-with-the-generated-module',
  deployment: {
    deploymentId: process.env.ARETE_DEPLOYMENT_ID ?? 'replace-with-the-generated-module',
    access: 'private',
    apiUrl: 'https://api.arete.run/api',
    sessionEndpoint: 'https://api.arete.run/ws/sessions',
  },
  instanceVisibility: 'shared',
  steps: {
    contribute: { scope: 'participant', access: 'public', guards: ['raise_not_open'], args: ['amount'] },
    claim: { scope: 'participant', access: 'public', guards: ['raise_not_finalized'], args: [] },
    refund: { scope: 'participant', access: 'public', guards: ['raise_not_failed'], args: [] },
  },
  events: {
    'raise.succeeded': { machine: 'instance' },
    'raise.failed': { machine: 'instance' },
    'contribution.confirmed': { machine: 'participant' },
  },
  metadata: {},
  stacks: {},
  programs: {},
};
