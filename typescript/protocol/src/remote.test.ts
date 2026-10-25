import { describe, expect, it } from "vitest";

import { checkRemotely, remoteCheckTarget } from "./remote.js";

const target = { apiUrl: "https://api.example.test", apiKey: "a4_sk_test" };

function fakeFetch(status: number, body: unknown, seen: Array<{ url: string; init: RequestInit }> = []): typeof fetch {
  return (async (url: string, init: RequestInit) => {
    seen.push({ url, init });
    return new Response(body === undefined ? "" : JSON.stringify(body), { status });
  }) as unknown as typeof fetch;
}

describe("remoteCheckTarget", () => {
  it("needs a secret key and defaults the API", () => {
    expect(remoteCheckTarget({})).toEqual({ reason: "no ARETE_API_KEY" });
    expect(remoteCheckTarget({ ARETE_API_KEY: "a4_pk_browser" })).toHaveProperty("reason");
    expect(remoteCheckTarget({ ARETE_API_KEY: "a4_sk_1" })).toEqual({
      apiUrl: "https://api.arete.run",
      apiKey: "a4_sk_1",
    });
    expect(remoteCheckTarget({ ARETE_API_KEY: "a4_sk_1", ARETE_API_URL: "http://localhost:3000/api/" })).toEqual({
      apiUrl: "http://localhost:3000",
      apiKey: "a4_sk_1",
    });
  });
});

describe("checkRemotely", () => {
  it("posts the upload body with the secret key", async () => {
    const seen: Array<{ url: string; init: RequestInit }> = [];
    const outcome = await checkRemotely(
      { spec: { name: "tips" }, bundleBase64: "ZXhwb3J0IHt9Owo=" },
      target,
      fakeFetch(200, { releaseHash: "arete:h1:protocol-release:sha256:00", templateCheck: "passed" }, seen),
    );
    expect(outcome).toEqual({
      kind: "passed",
      releaseHash: "arete:h1:protocol-release:sha256:00",
      templateCheck: "passed",
      missingProgramSpecs: [],
    });
    expect(seen[0]!.url).toBe("https://api.example.test/api/protocols/check");
    expect(seen[0]!.init.method).toBe("POST");
    expect((seen[0]!.init.headers as Record<string, string>).authorization).toBe("Bearer a4_sk_test");
    expect(JSON.parse(seen[0]!.init.body as string)).toEqual({
      spec: { name: "tips" },
      bundleBase64: "ZXhwb3J0IHt9Owo=",
    });
  });

  it("returns the server's errors, including a template check's nested list", async () => {
    const expression = await checkRemotely(
      { spec: {} },
      target,
      fakeFetch(400, {
        error: "The spec's expressions do not type-check",
        code: "protocol-spec-check-failed",
        errors: [{ code: "unknown-reference", path: "/steps/open/guards/0/expr", message: "unknown 'args.amout'" }],
      }),
    );
    expect(expression).toEqual({
      kind: "rejected",
      status: 400,
      code: "protocol-spec-check-failed",
      error: "The spec's expressions do not type-check",
      errors: [{ code: "unknown-reference", path: "/steps/open/guards/0/expr", message: "unknown 'args.amout'" }],
    });
    const template = await checkRemotely(
      { spec: {} },
      target,
      fakeFetch(400, {
        error: "The spec's templates do not resolve against their pinned ProgramSpecs",
        code: "protocol-template-check-failed",
        templateCheck: { errors: [{ code: "template_unknown_instruction", path: "/steps/open/build/instructions/0/instruction", message: "no 'withdraw'" }] },
      }),
    );
    expect(template.kind === "rejected" && template.errors[0]!.code).toBe("template_unknown_instruction");
  });

  it("reports a server without the check route as unavailable", async () => {
    expect((await checkRemotely({ spec: {} }, target, fakeFetch(404, undefined))).kind).toBe("unavailable");
    // A coded 404 is a real answer (the control plane is off, say).
    const coded = await checkRemotely(
      { spec: {} },
      target,
      fakeFetch(404, { error: "disabled", code: "protocol-control-plane-disabled" }),
    );
    expect(coded.kind).toBe("rejected");
  });

  it("does not pass a successful answer that is not a check result", async () => {
    for (const body of [undefined, {}, { releaseHash: "", templateCheck: "passed" }, { releaseHash: "h", templateCheck: "failed" }]) {
      const outcome = await checkRemotely({ spec: {} }, target, fakeFetch(200, body));
      expect(outcome.kind, JSON.stringify(body)).toBe("unavailable");
    }
    const html = (async () => new Response("<!doctype html><p>proxy</p>", { status: 200 })) as unknown as typeof fetch;
    const outcome = await checkRemotely({ spec: {} }, target, html);
    expect(outcome).toEqual({
      kind: "unavailable",
      reason: expect.stringContaining("unexpected answer from https://api.example.test/api/protocols/check"),
    });
  });

  it("gives up when the server stalls, on the request or its body", async () => {
    const stalledRequest = ((_url: string, init: RequestInit) =>
      new Promise((_resolve, reject) => {
        init.signal!.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
      })) as unknown as typeof fetch;
    const request = await checkRemotely({ spec: {} }, target, stalledRequest, 20);
    expect(request).toEqual({ kind: "unavailable", reason: expect.stringContaining("did not answer within 0.02s") });

    const stalledBody = (async (_url: string, init: RequestInit) =>
      new Response(
        new ReadableStream({
          start(controller) {
            controller.enqueue(new TextEncoder().encode("{"));
            init.signal!.addEventListener("abort", () => controller.error(new DOMException("aborted", "AbortError")));
          },
        }),
        { status: 200 },
      )) as unknown as typeof fetch;
    const body = await checkRemotely({ spec: {} }, target, stalledBody, 20);
    expect(body.kind).toBe("unavailable");
  });
});
