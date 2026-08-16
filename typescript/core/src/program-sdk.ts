import type {
  ProgramReadDescriptor,
  ProgramSdkDefinition,
} from './types';

/** Opaque generated-program metadata used to carry its default read transport. */
export const PROGRAM_READ_DESCRIPTOR: unique symbol = Symbol.for(
  '@usearete/sdk/program-read-descriptor',
) as any;

interface ProgramReadDescriptorCarrier {
  readonly [PROGRAM_READ_DESCRIPTOR]?: ProgramReadDescriptor;
}

/**
 * Bundle a generated program definition with its default read descriptor.
 *
 * Generated SDK entries call this automatically. The descriptor is deliberately
 * non-enumerable so it remains an implementation detail of the program cartridge.
 */
export function withProgramRead<
  TProgram extends ProgramSdkDefinition,
>(
  program: TProgram,
  descriptor: ProgramReadDescriptor,
): TProgram {
  const properties = Object.getOwnPropertyDescriptors(program);
  Reflect.deleteProperty(properties, PROGRAM_READ_DESCRIPTOR);
  const bundled = Object.create(
    Object.getPrototypeOf(program),
    properties,
  ) as TProgram;
  Object.defineProperty(bundled, PROGRAM_READ_DESCRIPTOR, {
    value: descriptor,
    enumerable: false,
    configurable: false,
    writable: false,
  });
  return bundled;
}

/** Resolve the default read descriptor carried by a generated program SDK. */
export function getProgramReadDescriptor(
  program: ProgramSdkDefinition | undefined,
): ProgramReadDescriptor | undefined {
  return (program as (ProgramSdkDefinition & ProgramReadDescriptorCarrier) | undefined)
    ?.[PROGRAM_READ_DESCRIPTOR];
}
