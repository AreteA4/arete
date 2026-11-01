import { extendProgram, withProgramIdentity, withProgramRead } from '@usearete/sdk/program';

import { SUBSCRIPTIONS as SUBSCRIPTIONS_PROGRAM_CORE, SUBSCRIPTIONS_READ as SUBSCRIPTIONS_PROGRAM_READ_CORE, SUBSCRIPTIONS_INSTRUCTION_SHAPES } from './subscriptions-core';
import programExtensions from './subscriptions-extensions';

export * from './subscriptions-core';
export { SUBSCRIPTIONS as SUBSCRIPTIONS_PROGRAM_CORE } from './subscriptions-core';

/** How this project installed the program (`programInstall` on SUBSCRIPTIONS_PROGRAM). */
export const SUBSCRIPTIONS_PROGRAM_INSTALL = {
  alias: 'subscriptions',
  programReleaseHash: 'arete:h1:program-release:sha256:6b4aab0353344e865113725a2737d41fa2416a25720bd8bb59c7d60698dabfdb',
  source: { kind: 'registry', registry: 'subscriptions', version: '1.0.13' },
} as const;

export type SubscriptionsProgram = ReturnType<typeof extendProgram<typeof SUBSCRIPTIONS_PROGRAM_CORE, typeof programExtensions>> & {
  readonly programInstall: typeof SUBSCRIPTIONS_PROGRAM_INSTALL;
  readonly instructionShapes: typeof SUBSCRIPTIONS_INSTRUCTION_SHAPES;
};

export const SUBSCRIPTIONS_PROGRAM: SubscriptionsProgram = Object.assign(
  withProgramIdentity(
    withProgramRead(
      extendProgram(SUBSCRIPTIONS_PROGRAM_CORE, programExtensions),
      SUBSCRIPTIONS_PROGRAM_READ_CORE,
    ),
    { packageReleaseHash: 'arete:registry-package-release:v2:sha256:ed1887b70e263307f80dbf6706d7a1763dd63f3e0bceb222feda9b897d1b809e' },
  ),
  { programInstall: SUBSCRIPTIONS_PROGRAM_INSTALL, instructionShapes: SUBSCRIPTIONS_INSTRUCTION_SHAPES },
);
export const SUBSCRIPTIONS_PROGRAM_READ = SUBSCRIPTIONS_PROGRAM_READ_CORE;

export default SUBSCRIPTIONS_PROGRAM;
