/** A request the handler host refuses, with an HTTP status and a stable code. */
export class HostError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "HostError";
    this.status = status;
    this.code = code;
  }
}

export function isPlainObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

/**
 * Check a request object's fields: it must be an object with every
 * `required` field and nothing outside `required` and `optional`.
 */
export function exactFields(
  value: unknown,
  what: string,
  required: readonly string[],
  optional: readonly string[] = [],
): Record<string, unknown> {
  if (!isPlainObject(value)) throw new HostError(400, "invalid_request", `${what} must be a JSON object`);
  for (const key of Object.keys(value)) {
    if (!required.includes(key) && !optional.includes(key)) {
      throw new HostError(400, "invalid_request", `${what} has unknown field '${key}'`);
    }
  }
  for (const key of required) {
    if (!Object.prototype.hasOwnProperty.call(value, key)) {
      throw new HostError(400, "invalid_request", `${what} is missing field '${key}'`);
    }
  }
  return value;
}
