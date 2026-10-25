# Dev environment switch vectors (v1)

Shared vectors for the switch between production and a cloud dev
environment. The contract is
[`docs/internal/dev-environment-switch.md`](../../../docs/internal/dev-environment-switch.md);
every SDK (TypeScript, React, Rust, Python), the `a4 dev up` edge and `a4`
itself run these files.

Each file is `{ "version": 1, "description": "...", "cases": [{ "name", "input", "expect" }] }`.
Compare `expect` exactly, except that a missing `message` or `fix` in an
expected refusal means "any". An expected refusal's `details` is exact too: it
lists every detail field the answer carries besides `message` and `fix`.

## Files

| File | What it covers |
|---|---|
| `switch.json` | Option, variables, session file and secret key in; `production`, dev through the edge, dev direct, or an error out. |
| `targets.json` | Installed descriptors and the mode (and, for direct mode, a resolve answer) in; each surface's endpoint, token request and session endpoint out, plus the resolve request direct mode sends. |
| `refusals.json` | Every refusal code, from the switch, the edge, resolve and service answers, and the SDK error each becomes. No case may lead to a request to the other world. |

## `switch.json` inputs

```json
{
  "browser": false,
  "option": null,
  "variables": { "ARETE_ENVIRONMENT": "dev", "ARETE_DEV_URL": "http://127.0.0.1:8911" },
  "sessionFile": { "edgeUrl": "http://127.0.0.1:8911" },
  "secretKey": true
}
```

- `option` is the client option exactly as an app passes it (`null` when
  absent), including malformed values.
- `variables` is the whole process environment the SDK may read. In a
  browser (`browser: true`) it must read none of it.
- `sessionFile` is the nearest `.arete/dev/session.json` (`null`: none).
- `secretKey` says whether the SDK's normal key resolution finds a secret
  key.

`expect` is `{ "mode": "production", "devInputsRead": false }`,
`{ "mode": "dev", "via": "edge", "edgeUrl" }`,
`{ "mode": "dev", "via": "direct", "environment" }` or
`{ "error": { "code" } }`. `devInputsRead: false` means the implementation
reaches the answer without reading `ARETE_DEV_URL`, `ARETE_DEV_ENV` or the
session file: run the case with a lookup that records what is read.

## `targets.json`

`input.installed` names one of the descriptors in the file's top-level
`installed` map: a Solana gateway binding, one program (`klend`) with a
hosted Program Read binding, one stack release and one protocol. `expect`
lists, per surface:

- `endpoint` (or `websocketUrl`, `apiUrl`);
- `tokenRequest`: the body sent to `sessionEndpoint`, or `null` when no token
  is minted and no `Authorization` header is sent;
- `pathReleaseHash`: the release hash in the Program Read request path;
- for the stack, `queryBaseUrl` and `queryParams`: where stack HTTP queries
  go and the query parameters added to each (`{}`: none);
- for the protocol, `deploymentPathSegment` (the deployment id, or
  `package:<package>` through the edge before the first publish) and
  `instancesUrl`, the URL of one real request (listing instances);
- `resolveRequest`: what direct mode sends to resolve (`null`: no request).

## `refusals.json`

`input.source` is `switch` (run the switch on `input.switch`), `edge` (the
edge's answer to the first request), `resolve` (direct mode with
`input.resolveResponse`) or `service` (`input.response` is a service's HTTP
answer to the first request on that surface). `expect.code` is the SDK
error's code; `fix` and `details` are in its details. `failingSurface` names
the surface that fails, and `otherSurfacesUsable` that the rest still work.
`productionRequests: 0` (or `devRequests: 0`) means the SDK makes no request
to the other world afterwards. An edge answer with a `request` is the answer to
that request (`GET /.well-known/arete-dev-edge`, the check before a socket),
`input.surface` names the surface being used, and `socketsOpened: 0` means no
WebSocket is opened.
