/**
 * A step's params schema as form fields, and form values back to step args.
 * Pure, so the form logic is tested without a browser.
 */

import type { FieldSchema, ObjectSchema } from "../types";

export type InputKind = "pubkey" | "integer" | "number" | "boolean" | "enum" | "string" | "json";

export interface FieldInput {
  name: string;
  field: FieldSchema;
  required: boolean;
  kind: InputKind;
  /** What the input shows when empty. */
  placeholder: string;
}

const INTEGER_FORMATS = new Set(["u8", "u16", "u32", "u64", "u128", "i64", "unixTimestamp"]);
const BASE58 = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/;

export function inputKind(field: FieldSchema): InputKind {
  if (field.enum?.length) return "enum";
  if (field.format === "pubkey") return "pubkey";
  if (field.format && INTEGER_FORMATS.has(field.format)) return "integer";
  switch (field.type) {
    case "integer":
      return "number";
    case "boolean":
      return "boolean";
    case "array":
    case "object":
      return "json";
    default:
      return "string";
  }
}

export function fieldInputs(schema: ObjectSchema): FieldInput[] {
  return Object.entries(schema.properties).map(([name, field]) => {
    const kind = inputKind(field);
    const placeholder =
      kind === "pubkey"
        ? "base58 public key"
        : kind === "integer"
          ? `${field.format} (a decimal string)`
          : kind === "json"
            ? field.type === "array"
              ? "[ … ] (JSON)"
              : "{ … } (JSON)"
            : (field.description ?? field.type);
    return { name, field, required: schema.required.includes(name), kind, placeholder };
  });
}

/** Starting values: empty, enums at their first value, booleans false. */
export function initialValues(schema: ObjectSchema): Record<string, string> {
  return Object.fromEntries(
    fieldInputs(schema).map((input) => [
      input.name,
      input.kind === "enum" ? (input.field.enum?.[0] ?? "") : input.kind === "boolean" ? "false" : "",
    ]),
  );
}

/** Step args from form values, or the errors that stop them. */
export function buildArgs(
  schema: ObjectSchema,
  values: Record<string, string>,
): { args: Record<string, unknown>; errors: Record<string, string> } {
  const args: Record<string, unknown> = {};
  const errors: Record<string, string> = {};
  for (const input of fieldInputs(schema)) {
    const raw = (values[input.name] ?? "").trim();
    if (raw === "" && input.kind !== "boolean") {
      if (input.required) errors[input.name] = "required";
      continue;
    }
    switch (input.kind) {
      case "pubkey":
        if (!BASE58.test(raw)) errors[input.name] = "not a base58 public key";
        else args[input.name] = raw;
        break;
      case "integer": {
        const signed = input.field.format === "i64";
        if (!(signed ? /^-?\d+$/ : /^\d+$/).test(raw)) errors[input.name] = signed ? "an integer" : "a non-negative integer";
        else args[input.name] = raw;
        break;
      }
      case "number": {
        const value = Number(raw);
        if (!Number.isInteger(value)) errors[input.name] = "an integer";
        else if (input.field.minimum !== undefined && value < input.field.minimum) errors[input.name] = `at least ${input.field.minimum}`;
        else if (input.field.maximum !== undefined && value > input.field.maximum) errors[input.name] = `at most ${input.field.maximum}`;
        else args[input.name] = value;
        break;
      }
      case "boolean":
        args[input.name] = raw === "true";
        break;
      case "enum":
        if (!input.field.enum?.includes(raw)) errors[input.name] = `one of ${input.field.enum?.join(", ")}`;
        else args[input.name] = raw;
        break;
      case "json":
        try {
          const parsed: unknown = JSON.parse(raw);
          const ok = input.field.type === "array" ? Array.isArray(parsed) : typeof parsed === "object" && parsed !== null && !Array.isArray(parsed);
          if (!ok) errors[input.name] = `a JSON ${input.field.type}`;
          else args[input.name] = parsed;
        } catch {
          errors[input.name] = "not JSON";
        }
        break;
      case "string":
        if (input.field.maxLength !== undefined && raw.length > input.field.maxLength) errors[input.name] = `at most ${input.field.maxLength} characters`;
        else args[input.name] = raw;
        break;
    }
  }
  return { args, errors };
}

/**
 * The args the form holds so far, for the copied command: valid values as
 * the step takes them, anything else as typed, required fields still empty
 * as `<name>`, and optional empty fields left out.
 */
export function draftArgs(schema: ObjectSchema, values: Record<string, string>): Record<string, unknown> {
  const { args } = buildArgs(schema, values);
  const draft: Record<string, unknown> = {};
  for (const input of fieldInputs(schema)) {
    const raw = (values[input.name] ?? "").trim();
    if (input.name in args) draft[input.name] = args[input.name];
    else if (raw !== "") draft[input.name] = raw;
    else if (input.required) draft[input.name] = `<${input.name}>`;
  }
  return draft;
}

/** The dev wallet a step's authority suggests for an instance (its role holder, or the creator). */
export function suggestedWallet(
  authority: { kind: string; role?: string },
  instance: { roles?: Record<string, string>; accounts?: Record<string, string> } | undefined,
  wallets: Array<{ name: string; pubkey: string }>,
): string | undefined {
  const byKey = (pubkey: string | undefined) => wallets.find((wallet) => wallet.pubkey === pubkey)?.name;
  if (authority.kind === "role" && authority.role) return byKey(instance?.roles?.[authority.role]);
  if (authority.kind === "creator" && instance) return byKey(instance.accounts?.creator);
  return undefined;
}
