import React, { type ReactNode } from 'react';
import type { ProgramSdkDefinition } from '@usearete/sdk';

import { AreteProvider } from './provider';
import { useArete, type UseAreteResult } from './stack';
import type { AreteConfig, StackDefinition, UseAreteOptions } from './types';

type ProgramMap = Record<string, ProgramSdkDefinition>;

export type BoundAreteProviderProps = Omit<AreteConfig, 'stack'> & {
  children: ReactNode;
};

/**
 * Bind the React SDK to one generated stack without ambient module
 * augmentation. The returned hook remains explicit through an app-specific
 * name such as `useOre`, while the generic provider and hook remain available
 * for multi-stack applications.
 */
export function createAreteReact<TStack extends StackDefinition>(stack: TStack) {
  function Provider({ children, ...config }: BoundAreteProviderProps) {
    return (
      <AreteProvider {...config} stack={stack}>
        {children}
      </AreteProvider>
    );
  }

  function useBoundArete<
    TPrograms extends ProgramMap | undefined = undefined,
  >(options?: UseAreteOptions<TPrograms>): UseAreteResult<TStack, TPrograms> {
    return useArete(stack, options);
  }

  return {
    Provider,
    useArete: useBoundArete,
    stack,
  } as const;
}
