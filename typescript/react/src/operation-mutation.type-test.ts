/**
 * Compile-time checks: a connected client and a session are both
 * `useOperationMutation` executors as they are. Checked by `tsc --noEmit`.
 */

import type { ConnectedArete, StackDefinition, createSession } from '@usearete/sdk';
import type { OperationExecutor, OperationExecutorSource } from './hooks/use-operation-mutation';

type Session = Awaited<ReturnType<typeof createSession>>;

declare const client: ConnectedArete<StackDefinition>;
declare const session: Session;
declare const openSession: () => Promise<Session>;

export const fromClient: OperationExecutor = client;
export const fromSession: OperationExecutor = session;
export const lazily: OperationExecutorSource = openSession;
