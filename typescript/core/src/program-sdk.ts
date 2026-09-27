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
 * Generated SDK entries call this automatically. The descriptor lives under a
 * registry symbol, so `Object.keys` and `JSON.stringify` never see it, while
 * `{ ...program }` keeps it: a spread program still reads through its release.
 * Every other own property (including the runtime extension carrier) is copied
 * unchanged, except `packageReleaseHash`: the result reads differently from
 * the generated program SDK, so it is no longer provably that SDK. Generated
 * entries apply `withProgramIdentity` last.
 */
export function withProgramRead<
  TProgram extends ProgramSdkDefinition,
>(
  program: TProgram,
  descriptor: ProgramReadDescriptor,
): TProgram {
  const properties = Object.getOwnPropertyDescriptors(program);
  Reflect.deleteProperty(properties, PROGRAM_READ_DESCRIPTOR);
  Reflect.deleteProperty(properties, 'packageReleaseHash');
  const bundled = Object.create(
    Object.getPrototypeOf(program),
    properties,
  ) as TProgram;
  Object.defineProperty(bundled, PROGRAM_READ_DESCRIPTOR, {
    value: descriptor,
    enumerable: true,
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

/** The identity a generated program SDK entry stamps on its final program. */
export interface ProgramSdkIdentity {
  /** The program package release the SDK was generated from, when known. */
  readonly packageReleaseHash?: string;
}

/**
 * Stamp a program definition with the identity of the program SDK it is.
 *
 * Generated program SDK entries call this last, after the package's own
 * extension and read descriptor are applied, so the identity describes
 * exactly the generated SDK. `extendProgram`, `extendPrograms` and
 * `withProgramRead` drop the identity again: a program changed outside its
 * generated SDK is no longer provably that SDK. Every other own property,
 * including symbol-keyed runtime extensions and the read descriptor, is copied
 * unchanged. An absent or empty `packageReleaseHash` removes the identity.
 */
export function withProgramIdentity<
  TProgram extends ProgramSdkDefinition,
>(
  program: TProgram,
  identity: ProgramSdkIdentity,
): TProgram {
  const properties = Object.getOwnPropertyDescriptors(program);
  Reflect.deleteProperty(properties, 'packageReleaseHash');
  const stamped = Object.create(
    Object.getPrototypeOf(program),
    properties,
  ) as TProgram;
  if (typeof identity.packageReleaseHash === 'string' && identity.packageReleaseHash.length > 0) {
    Object.defineProperty(stamped, 'packageReleaseHash', {
      value: identity.packageReleaseHash,
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  return stamped;
}
