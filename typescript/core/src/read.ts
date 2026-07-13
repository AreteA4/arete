import type {
  ProgramAccountReadDefinition,
  ProgramQueryDefinition,
  ReadTransportMethod,
  Schema,
  StackQueryDefinition,
} from './types';

export class ReadRequestError extends Error {
  readonly status: number;
  readonly path: string;
  readonly body: string;
  readonly serverErrorCode: string | undefined;

  constructor(input: {
    status: number;
    path: string;
    body: string;
    serverErrorCode?: string;
  }) {
    super(`Read request to '${input.path}' failed (${input.status}): ${input.body}`);
    this.name = 'ReadRequestError';
    this.status = input.status;
    this.path = input.path;
    this.body = input.body;
    this.serverErrorCode = input.serverErrorCode;
  }
}

function getServerErrorCode(response: Response, body: string): string | undefined {
  const headerCode = response.headers.get('X-Error-Code');
  if (headerCode) {
    return headerCode;
  }
  try {
    const parsed = JSON.parse(body) as { code?: unknown };
    return typeof parsed.code === 'string' ? parsed.code : undefined;
  } catch {
    return undefined;
  }
}

export async function parseReadResponse<T>(response: Response, path: string): Promise<T> {
  if (!response.ok) {
    const body = await response.text();
    throw new ReadRequestError({
      status: response.status,
      path,
      body,
      serverErrorCode: getServerErrorCode(response, body),
    });
  }
  return response.json() as Promise<T>;
}

export function programAccountRead<T>(input: {
  account: string;
  path: string;
  schema?: Schema<T>;
}): ProgramAccountReadDefinition<T> {
  return {
    account: input.account,
    path: input.path,
    schema: input.schema,
  } as const;
}

export function programQuery<TParams = unknown, TResult = unknown>(input: {
  name: string;
  path: string;
  method?: ReadTransportMethod;
  schema?: Schema<TResult>;
}): ProgramQueryDefinition<TParams, TResult> {
  return {
    name: input.name,
    path: input.path,
    method: input.method,
    schema: input.schema,
  } as const;
}

export function stackQuery<TParams = unknown, TResult = unknown>(input: {
  name: string;
  path: string;
  method?: ReadTransportMethod;
  schema?: Schema<TResult>;
}): StackQueryDefinition<TParams, TResult> {
  return {
    name: input.name,
    path: input.path,
    method: input.method,
    schema: input.schema,
  } as const;
}
