import { MY_STACK_PROGRAMS as MY_STACK_PROGRAMS_CORE } from './my.stack-programs-core.js';

export * from './my.stack-programs-core.js';

export const MY_STACK_PROGRAMS = MY_STACK_PROGRAMS_CORE;

export type MyStackPrograms = typeof MY_STACK_PROGRAMS;

export default MY_STACK_PROGRAMS;
