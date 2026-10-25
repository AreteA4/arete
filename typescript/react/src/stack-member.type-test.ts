// Type-level tests: `npm run typecheck` compiles this file; nothing runs it.
//
// `useArete(session.stacks.<key>)` is typed exactly as `useArete(stack)` is
// for that member's stack: views, programs and the connected client.

import {
  PROGRAM_OPERATION_EXTENSIONS,
  type InstructionOperation,
  type Session,
  type ViewDef,
} from '@usearete/sdk';

import { useArete, type UseAreteResult } from './stack';

type Equal<TLeft, TRight> =
  (<T>() => T extends TLeft ? 1 : 2) extends
  (<T>() => T extends TRight ? 1 : 2)
    ? true
    : false;
type Assert<T extends true> = T;

interface RoundEntity {
  roundId: bigint;
}
interface RoundKey {
  roundId: bigint;
}
interface DeployInput {
  amount: number;
}

declare const deploy: InstructionOperation<DeployInput, { squares: number }>;

const ORE = {
  name: 'ore',
  endpoints: { ws: 'wss://ore.invalid' },
  views: {
    Round: {
      state: { mode: 'state', view: 'Round/state' } as ViewDef<RoundEntity, 'state', RoundKey>,
      list: { mode: 'list', view: 'Round/list' } as ViewDef<RoundEntity, 'list'>,
    },
  },
  programs: {
    ore: {
      name: 'ore',
      programId: 'ore-program',
      [PROGRAM_OPERATION_EXTENSIONS]: {
        createOperations() {
          return { instructions: { deploy } };
        },
      },
    },
  },
} as const;

const DEFINITION = { stacks: { ore: ORE } };
declare const session: Session<typeof DEFINITION>;
declare const opening: Session<typeof DEFINITION> | null;

const member = useArete(session.stacks.ore);
const byStack = useArete(ORE);

export type MemberResultIsTheStackResult = Assert<Equal<typeof member, UseAreteResult<typeof ORE>>>;
export type MemberStateViewIsTyped = Assert<Equal<
  Parameters<typeof member.views.Round.state.use>[0],
  RoundKey | null | undefined
>>;
export type MemberStateDataIsTyped = Assert<Equal<
  NonNullable<ReturnType<typeof member.views.Round.state.use<RoundEntity>>['data']>,
  RoundEntity
>>;
export type MemberListViewIsTyped = Assert<Equal<
  typeof member.views.Round.list.useOne<RoundEntity>,
  typeof byStack.views.Round.list.useOne<RoundEntity>
>>;
export type MemberProgramIsTyped = Assert<Equal<
  Parameters<typeof member.programs.ore.instructions.deploy.execute>[0],
  DeployInput
>>;
export type MemberViewsMatchTheStack = Assert<Equal<typeof member.views, typeof byStack.views>>;
export type MemberProgramsMatchTheStack = Assert<Equal<typeof member.programs, typeof byStack.programs>>;
export type MemberClientIsTyped = Assert<Equal<
  NonNullable<typeof member.client>['views'],
  NonNullable<typeof byStack.client>['views']
>>;

// The member carries its stack definition and the transport its session
// connected it with; the hooks take both from it, so neither is an option.
export type MemberStackIsTyped = Assert<Equal<typeof session.stacks.ore.stack['views'], typeof ORE.views>>;
export type MemberTransportIsTyped = Assert<Equal<typeof session.stacks.ore.transport, 'ws' | 'http'>>;
// @ts-expect-error the member's transport comes from the member
useArete(session.stacks.ore, { transport: 'http' });

// While the session opens, the member is `null`/`undefined` and the stack
// definition gives the hooks their placeholders.
const pending = useArete(opening?.stacks.ore, { stack: ORE });
export type PendingMemberIsTyped = Assert<Equal<typeof pending, typeof member>>;

// @ts-expect-error a member that may be missing needs its stack definition
useArete(opening?.stacks.ore);
// @ts-expect-error a member's stack must have the member's views
useArete(opening?.stacks.ore, { stack: { name: 'other', endpoints: { ws: '' }, views: {} } });
// @ts-expect-error not a view of the member's stack
void member.views.Missing;

// A member with programs attached through the session's member options keeps
// them, and its generated stack still serves as the placeholder definition.
declare const MEMO: { readonly name: 'memo'; readonly programId: 'memo-program' };
declare const attached: Session<typeof DEFINITION, { ore: { memo: typeof MEMO } }> | null;
const withMemo = useArete(attached?.stacks.ore, { stack: ORE });
const withMemoByStack = useArete(ORE, { programs: { memo: MEMO } });
export type AttachedProgramIsTyped = Assert<Equal<
  typeof withMemo.programs.memo,
  typeof withMemoByStack.programs.memo
>>;
export type AttachedKeepsStackProgram = Assert<Equal<
  Parameters<typeof withMemo.programs.ore.instructions.deploy.execute>[0],
  DeployInput
>>;
