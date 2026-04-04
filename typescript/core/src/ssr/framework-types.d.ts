declare module 'next/server' {
  export interface NextRequest {
    headers: Headers;
  }
}

declare module 'express' {
  export interface Request {
    method: string;
    path: string;
    headers: Record<string, string | string[] | undefined> & {
      origin?: string;
    };
  }

  export interface Response {
    json(data: unknown): Response;
    status(code: number): Response;
  }
}
