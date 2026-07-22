import {
  useAsyncRead,
  type AsyncReadKey,
  type UseAsyncReadOptions,
  type UseAsyncReadResult,
} from './hooks/use-async-read';
import type { ReadArgumentCounts } from '@usearete/sdk';

type NullableArgs<TArgs extends readonly unknown[]> = {
  [I in keyof TArgs]: TArgs[I] | undefined | null;
};

type RequiredNullableArgs<TArgs extends readonly unknown[]> = {
  [I in keyof TArgs]-?: TArgs[I] | undefined | null;
};

type AnyReadRecord = Record<string, (...args: never[]) => unknown>;
const readSourceIds = new WeakMap<object, number>();
let nextReadSourceId = 0;

function readSourceId(read: object | undefined): number {
  if (!read) return 0;
  const existing = readSourceIds.get(read);
  if (existing !== undefined) return existing;
  const id = ++nextReadSourceId;
  readSourceIds.set(read, id);
  return id;
}

export type ReactReadInterface<TRead> = [TRead] extends [never]
  ? Record<string, never>
  : {
      [K in keyof TRead]: TRead[K] extends (...args: infer TArgs) => infer TReturn
        ? TArgs extends readonly unknown[]
          ? TRead[K] & {
              use: {
                (...args: NullableArgs<TArgs>): UseAsyncReadResult<Awaited<TReturn>>;
                (
                  ...args: [
                    ...RequiredNullableArgs<TArgs>,
                    ReadHookOptions<Awaited<TReturn>>,
                  ]
                ): UseAsyncReadResult<Awaited<TReturn>>;
              };
            }
          : never
        : never;
    };

/** @deprecated Use {@link ReactReadInterface}; `read` now includes both forms. */
export type ReadHookInterface<TRead> = ReactReadInterface<TRead>;

export type ReadHookOptions<T> = Pick<UseAsyncReadOptions<T>, 'initialData' | 'debounceMs'>;

/**
 * Wrap a stack's connected `read` functions in React hooks, mirroring how
 * program operations get `useMutation`. Each read exposes `.use(...args)`:
 * the args form the cache key and the read stays disabled while any required
 * arg is nullish, so `read.solClaimPreview.use(authority)` simply waits for a
 * connected wallet instead of needing non-null assertions and an `enabled`
 * flag.
 *
 * Hook-only options occupy the position after every declared read argument.
 * Optional read arguments must therefore be passed as `undefined` when options
 * follow them, keeping object-valued read arguments unambiguous.
 *
 * A proxy backs the interface so hooks exist (and stay disabled) before the
 * client connects — components can call them unconditionally.
 */
export function buildReadInterfaces<TRead extends AnyReadRecord>(
  read: TRead | null | undefined,
  readArgCounts: ReadArgumentCounts<TRead> | undefined,
): ReactReadInterface<TRead> {
  return new Proxy({} as ReactReadInterface<TRead>, {
    get: (_target, name) => {
      if (typeof name !== 'string') {
        return undefined;
      }
      const readFn = read?.[name];
      const sourceId = readSourceId(readFn);
      const readArgCount = readArgCounts?.[name];
      const [requiredArgCount, totalArgCount] = Array.isArray(readArgCount)
        ? readArgCount
        : [readArgCount, readArgCount];
      const useRead = (options: ReadHookOptions<unknown> | undefined, args: unknown[]) => {
        const hasArgs = args.length >= requiredArgCount
          && args
            .slice(0, requiredArgCount)
            .every((arg) => arg !== undefined && arg !== null);
        return useAsyncRead(
          [name, sourceId, ...args] as AsyncReadKey[],
          async () => {
            if (!readFn) {
              throw new Error('Arete client is not connected');
            }
            return readFn(...(args as never[]));
          },
          {
            ...options,
            enabled: readFn != null && hasArgs,
            disabledStatus: hasArgs && readFn == null ? 'connecting' : 'disabled',
          },
        );
      };
      const use = (...argsAndOptions: unknown[]) => {
        if (
          !Number.isInteger(requiredArgCount)
          || !Number.isInteger(totalArgCount)
          || requiredArgCount < 0
          || totalArgCount < requiredArgCount
        ) {
          throw new Error(`Missing read argument count metadata for "${name}"`);
        }
        let options: ReadHookOptions<unknown> | undefined;
        let args = argsAndOptions;
        if (argsAndOptions.length <= totalArgCount) {
          options = undefined;
        } else if (argsAndOptions.length !== totalArgCount + 1) {
          throw new Error(
            `Read hook "${name}" expects ${totalArgCount} argument(s) followed by optional hook options`,
          );
        } else {
          const candidate = argsAndOptions[totalArgCount];
          if (candidate === null || typeof candidate !== 'object' || Array.isArray(candidate)) {
            throw new Error(`Read hook options for "${name}" must be an object`);
          }
          options = candidate as ReadHookOptions<unknown>;
          args = argsAndOptions.slice(0, totalArgCount);
        }
        return useRead(options, args);
      };
      const imperative = (...args: unknown[]) => {
        if (!readFn) {
          throw new Error('Arete client is not connected');
        }
        return readFn(...(args as never[]));
      };
      return Object.assign(imperative, {
        use,
      });
    },
  });
}

/** @deprecated Use {@link buildReadInterfaces}. */
export const buildReadHookInterfaces = buildReadInterfaces;
