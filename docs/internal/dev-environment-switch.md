# Dev environment switch — SDK contract (v1)

Status: **contract**. Every Arete SDK (TypeScript, React, Rust, Python), the
`a4 dev up` loopback edge, `a4` itself and the Arete API implement this. Change
it here first, then in code. Shared vectors:
[`test-vectors/dev-environment-switch/v1/`](../../test-vectors/dev-environment-switch/v1/README.md).

An app uses the SDKs it installed from the registry (`a4 install program|stack|protocol`).
One switch decides where those SDKs go:

- **Production** (the default): the generated descriptors exactly as they are.
- **Dev**: the project's cloud dev environment (a fork of mainnet). The same
  generated files read and send through the environment's fork, read program
  accounts against the fork, stream the environment's stacks and call the
  environment's protocol deployments.

Nothing is regenerated for the switch. Generated files never contain a dev URL.

## 1. Rules that hold everywhere

1. **Production needs nothing.** No switch set, or `production`, is the code
   path of today. With the switch unset, an SDK makes byte-identical requests
   to the ones it made before this contract, and reads no other dev
   configuration (`ARETE_DEV_URL`, `ARETE_DEV_ENV`, `.arete/dev/session.json`).
2. **Fail closed both ways.** In dev mode, a surface the environment can't
   serve is an error with a fix, never a production read or send. In
   production mode, no dev URL or environment token is ever used. A switch
   value the SDK doesn't recognise is an error.
3. **No fallback.** An SDK never retries a refused dev request against
   production, and never retries a refused production request against an
   environment.
4. **Resolve by identity.** The SDK sends installed identities and receives
   the environment's targets (§4). It never derives a dev URL from a
   production one.
5. **Every environment token names its environment** (§5).
6. **Browsers never hold an Arete key in dev mode.** The browser path is the
   edge (§3.1): a loopback process that holds the key and attaches tokens.

## 2. The switch

### 2.1 Inputs

| Input | Where | Meaning |
|---|---|---|
| `environment` option | every client constructor; `<AreteProvider environment>` | `'production'`, or `{ kind: 'dev', url?: string, env?: string }` |
| `ARETE_ENVIRONMENT` | process environment, **outside browsers only** | `production` or `dev` |
| `ARETE_DEV_URL` | process environment, outside browsers only | the edge origin `a4 dev up` serves |
| `ARETE_DEV_ENV` | process environment, outside browsers only | an environment id (`denv_…`) or name, for direct mode |
| `.arete/dev/session.json` `edgeUrl` | the nearest file at or above the working directory, outside browsers only | the running edge, as `a4 dev up` writes it |
| a secret key | `ARETE_API_KEY` or the `a4` login (the SDK's existing key resolution) | needed for direct mode only |

Browser apps pass the option from their framework's public variables, for
example `environment: import.meta.env.VITE_ARETE_ENVIRONMENT === 'dev' ? { kind: 'dev', url: import.meta.env.VITE_ARETE_DEV_URL } : 'production'`.
The edge URL is not a secret.

### 2.2 Grammar

- `ARETE_ENVIRONMENT`: exactly `production` or `dev` (lowercase, no
  surrounding whitespace). Unset or the empty string is `production`. Any
  other value is `INVALID_CONFIG`.
- `environment` option: the string `'production'`, or an object whose `kind`
  is `'dev'` and whose only other members are `url` and `env` (strings).
  Anything else is `INVALID_CONFIG`.
- An edge URL is `http://` or `https://`, a loopback host (`127.0.0.1`,
  `[::1]` or `localhost`), an explicit port, and no path other than `/`, no
  query, fragment or user information. Anything else is `INVALID_CONFIG`.
- An environment reference is an id (`denv_` followed by 26 lowercase
  Crockford base32 characters, the first at most `7`) or a name (1–63 of
  `a-z`, `0-9` and `-`, as `a4 dev` names environments). Anything else is
  `INVALID_CONFIG`.

### 2.3 Precedence

1. **The option wins.** `environment: 'production'` is production whatever
   the variables say. `environment: { kind: 'dev', … }` is dev.
2. **Else `ARETE_ENVIRONMENT`**, outside browsers. In a browser no variable is
   read and the default is production.
3. **Else production.**

In dev mode the target is the first of these that is set:

1. the option's `url` → **edge** mode;
2. the option's `env` → **direct** mode;
3. `ARETE_DEV_URL` (outside browsers) → edge;
4. `ARETE_DEV_ENV` (outside browsers) → direct;
5. `edgeUrl` from the nearest `.arete/dev/session.json` (outside browsers) → edge.

Explicit values beat the discovered session file, so a CI job with
`ARETE_DEV_ENV` is not captured by a stale file. When none is set the SDK
fails with `devenv-not-configured`.

Direct mode needs a secret key and never runs in a browser. Without a secret
key, or in a browser, direct mode fails with `devenv-not-configured`.

### 2.4 Result

```ts
type SwitchResult =
  | { mode: 'production' }
  | { mode: 'dev'; via: 'edge'; edgeUrl: string }        // normalized: no trailing slash
  | { mode: 'dev'; via: 'direct'; environment: string };  // as given: id or name
```

Every client exposes `client.environment`: `'production' | 'dev'`. It is the
one value apps and `@usearete/automations` read to tell dev from production.

The switch is decided once per client, at construction. A client never changes
mode.

## 3. Endpoints per mode

### 3.1 Edge

`a4 dev up` serves the edge on loopback. It accepts only requests that name it
by a loopback host and, from browsers, only a loopback `Origin`. It holds the
account's key, mints environment tokens itself and never returns a key or a
token. A page or process using the edge needs no Arete credential.

| Surface | Installed descriptor | In edge mode |
|---|---|---|
| Chain reads | `gateway.chain.endpoint` | `<edge>`, same paths (`/chain/…`) |
| Transactions | `gateway.transactions.endpoint` | `<edge>`, same paths (`/transactions/v1/…`) |
| Program Read | `programReads.<p>.transport.binding.endpoint` | `<edge>`, same path `/v1/releases/<installed release>/…` |
| Stack | `endpoints.ws` + `release` | `ws(s)://<edge host>/stacks/<liveAlias>?stackManifestHash=<hash>` |
| Stack queries | `endpoints.http` + `release` | `<edge>/stacks/<liveAlias>`, same paths, with `stackManifestHash=<hash>` added to the query string |
| Protocol API and sessions | `deployment.apiUrl`, `deployment.sessionEndpoint` | `<edge>/api`, `<edge>/ws/sessions`, same deployment id |
| JSON-RPC, subscriptions | — | `<edge>` (POST) and `<edge>` upgraded to WebSocket |

In edge mode the SDK sends **no** `Authorization` header and mints no token:
auth metadata in the descriptor is ignored. The edge resolves installed
identities (§4) and answers with the production-shaped body, or with a refusal
(§6). The installed identities (release hash, manifest hash, deployment id)
stay in the request exactly as generated; the edge rewrites them.

A path does not carry every identity resolve needs (a Program Read path has
the release but not the binding; a chain path has neither). So every edge
request except a stack's sends `X-Arete-Dev-Identity`: the resolve request
(§4.1) for that one surface and that one identity, as compact JSON. It is the
same object direct mode sends to resolve:

| Surface | `X-Arete-Dev-Identity` |
|---|---|
| Chain reads | `{"chain":{"solanaGatewayBindingId":"sgb_…"}}` |
| Transactions | `{"transactions":{"solanaGatewayBindingId":"sgb_…"}}` |
| Program Read | `{"programReads":[{"programId":"…","programReadBindingId":"prb_…","programReleaseHash":"<installed>"}]}` |
| Protocol API | `{"protocols":[{"package":"…","protocolReleaseHash":"…","deploymentId":"…"}]}` (no `deploymentId` before the first publish) |
| Stack | none: the path and query name the live alias and manifest |

The header carries identities, never a credential. An edge request on one
of those surfaces without it is refused with `devenv-surface-unavailable`.

A protocol client with no deployment id yet (before the first publish) names
the protocol by package instead: wherever the protocol API path has the
deployment id segment (`/api/protocol-deployments/{deploymentId}/…`), it
sends `package:<package>`, percent-encoded as one segment
(`/api/protocol-deployments/package:token-raise/instances`). The edge maps it
to the environment's deployment of that package, so two unpublished
protocols behind one edge stay apart. Only the edge accepts this form; in
direct mode resolve returns a deployment id (§4).

Every answer the edge forwards on a `/api/protocol-deployments/…` route names
the environment's deployment it called in `X-Arete-Dev-Deployment`. The
installed descriptor never carries it (it names the production deployment, or
none before the first publish), yet two things need it: a metadata wallet
proof signs the deployment id, and the API checks it against the instance's
own deployment; automations bind their store and match webhook deliveries by
it. A protocol client takes it from that header (a `GET` of the deployment
route when it has made no call yet), in direct mode from resolve, and only
then builds a proof. In production the generated deployment is used; a
descriptor without one fails with "not deployed to production yet".

Every edge HTTP response carries `X-Arete-Dev-Edge: 1` and, for loopback
origins, `Access-Control-Expose-Headers` naming it and
`X-Arete-Dev-Deployment`, so browser `fetch` can read them. WebSocket clients can't read upgrade response headers,
so before a client opens its first socket (stack streams, JSON-RPC
subscriptions) it checks the edge once with
`GET <edge>/.well-known/arete-dev-edge`. The edge answers `200` with
`{"edge": 1}` and the header. Any other answer fails the socket with
`devenv-edge-unreachable`, and no socket is opened.

The edge also answers `POST /ws/sessions` with a token that only the edge
accepts, for code paths that always mint; SDKs implementing this contract
don't call it.

A stack definition without `release` (not installed from the registry) has no
identity to resolve and fails with `devenv-stack-not-loaded`.

If the edge does not answer (connection refused, a response without the
`X-Arete-Dev-Edge: 1` header, or a failed `/.well-known/arete-dev-edge`
check before a socket), the SDK fails with `devenv-edge-unreachable`. It
never falls back.

### 3.2 Direct

For servers, CI and agents with a secret key. The SDK calls resolve (§4) once
per client for the identities it was built with, then uses the returned
targets:

| Surface | In direct mode |
|---|---|
| Chain reads | `chain.endpoint`, same paths |
| Transactions | `transactions.endpoint`, same paths |
| Program Read | `programReads[i].endpoint`, path `/v1/releases/<programReads[i].programReleaseHash>/…` |
| Stack | `stacks[i].websocketUrl` |
| Stack queries | `stacks[i].httpUrl`, same paths, with the stack's environment token; no `httpUrl` in the answer fails queries (not the stream) with `devenv-surface-unavailable` |
| Protocol | `protocols[i].deploymentId`, `apiUrl`, `sessionEndpoint` |

Tokens come from the same session endpoint the descriptor names
(`https://api.arete.run/ws/sessions`), with the installed request body plus
`environment` (§5). Hostnames under `.devstack.arete.run` count as hosted
WebSocket endpoints (a session is required) in direct mode only.

### 3.3 Production

The generated descriptors as they are. `.devstack.arete.run` is not a hosted
suffix, and no `environment` is ever sent.

## 4. Resolve

`POST https://api.arete.run/api/dev/environments/{env}/resolve`, with
`Authorization: Bearer <secret key>`. `{env}` is an id or name. Only the
environment's owner may call it; anyone else gets `404 devenv-not-found`.
A publishable key is refused (`403 SECRET_KEY_REQUIRED`).

### 4.1 Request

Every member is optional; send the identities the client was built with.

```json
{
  "chain": { "solanaGatewayBindingId": "sgb_…" },
  "transactions": { "solanaGatewayBindingId": "sgb_…" },
  "programReads": [
    { "programId": "KLend2g3…", "programReadBindingId": "prb_…", "programReleaseHash": "arete:h1:program-release:sha256:…" }
  ],
  "stacks": [
    { "stackManifestHash": "arete:h1:stack-manifest:sha256:…", "liveAlias": "default" }
  ],
  "protocols": [
    { "package": "token-raise", "protocolReleaseHash": "…", "deploymentId": "…" }
  ]
}
```

A protocol has no `deploymentId` before its first publish; `package` and
`protocolReleaseHash` are always sent.

### 4.2 Response

`200` with one answer per requested identity, in request order. Each answer is
either a target or a refusal:

```json
{
  "environment": { "id": "denv_…", "name": "my-env" },
  "chain": {
    "status": "target",
    "endpoint": "https://devenv.arete.run/devenv/v1/denv_…"
  },
  "transactions": { "status": "target", "endpoint": "https://devenv.arete.run/devenv/v1/denv_…" },
  "programReads": [
    {
      "status": "target",
      "endpoint": "https://devenv.arete.run",
      "programReleaseHash": "arete:h1:program-release:sha256:…",
      "releaseDiffers": false
    }
  ],
  "stacks": [
    {
      "status": "target",
      "websocketUrl": "wss://s…-….devstack.arete.run",
      "httpUrl": "https://s…-….devstack.arete.run",
      "stackManifestHash": "arete:h1:stack-manifest:sha256:…",
      "releaseDiffers": false,
      "source": "manifest",
      "loadedAtSlot": 412345678
    }
  ],
  "protocols": [
    {
      "status": "refused",
      "code": "devenv-protocol-not-deployed",
      "message": "token-raise is not deployed in my-env",
      "fix": "Run `a4 dev up` in the protocol's project to deploy it into the environment."
    }
  ]
}
```

- `releaseDiffers` is `true` when the environment serves another release than
  the installed one (a program changed in the environment and registered as an
  override, a project stack loaded under the dependency's alias, a protocol
  built locally). The SDK uses the returned release; types still follow the
  installed SDK.
- `stacks[i].httpUrl` is where the stack answers HTTP queries
  (`endpoints.http` in production). It is absent when the environment's stack
  serves only the stream.
- `stacks[i].source` is `override` (a stack loaded under the dependency's
  alias) or `manifest` (the environment's deployment of exactly that
  manifest). `loadedAtSlot` is the slot it started streaming from: a stack in
  an environment holds only accounts the environment touched from then on.
- No answer ever contains a production endpoint or a production target id.

### 4.3 Using the answer

- Any `refused` answer for a surface the client uses fails that surface with
  the answer's `code`, `message` and `fix` (§6). Other surfaces keep working.
- A surface whose identity was not sent is not available in dev mode and fails
  with `devenv-surface-unavailable` when used.
- Clients may cache an answer for the life of the client. A refusal from a
  target later (§6) does not trigger a re-resolve against production.

## 5. Environment tokens

Direct mode mints from the session endpoint with the installed request and
`environment`:

```json
{ "targetKind": "solana-gateway-binding", "targetId": "sgb_…", "scopes": ["read"], "environment": "denv_…" }
{ "targetKind": "program-read-binding", "targetId": "prb_…", "programReleaseHash": "<installed>", "scopes": ["read"], "environment": "denv_…" }
{ "websocket_url": "<stacks[i].websocketUrl>", "scopes": ["read"], "stackManifestHash": "<served>", "liveAlias": "…", "environment": "denv_…" }
```

The response shape is unchanged (`{ token, expires_at, … }`). The caller must
own the environment and authenticate with a secret key. The session endpoint
resolves the installed target to the environment's target itself, so the SDK
never names an environment target id.

Protocol sessions are unchanged: they name the environment's deployment id
from resolve, and the token is bound to that deployment's environment.

Every token minted for an environment carries an `env` claim naming it.
Production services (`solana.arete.run`, `programs.arete.run`,
`*.stack.arete.run`, protocol calls on production deployments) refuse any
token with an `env` claim (`403 environment-token-refused`). Dev services
(`devenv.arete.run`, `*.devstack.arete.run`) refuse a token without one
(`401 environment-token-required`) and a token for another environment
(`403 environment-token-refused`).

## 6. Refusals

Refusals from the API (resolve, sessions), the edge and `devenv.arete.run` use
the API problem envelope (`schemaVersion`, `error`, `code`, `retryable`) plus
`fix`: one sentence naming the command or change that resolves it. Program
Read keeps its own envelope, `{"error": {"code", "detail": {"message", "fix",
…}}}`, with the code also in `X-Error-Code`. SDKs read either and raise their
error type with `code` equal to the wire code, the message, and `fix` and
every other detail field in the error details.

| Code | Raised by | HTTP | Meaning | Fix |
|---|---|---|---|---|
| `devenv-not-configured` | SDK | — | Dev mode, but no edge URL or environment (or direct mode without a secret key, or in a browser). | Run the app with `a4 dev run -- <command>`, start `a4 dev up`, or set `ARETE_DEV_ENV` with a secret key. |
| `devenv-edge-unreachable` | SDK | — | The edge URL does not answer as an edge. | Start `a4 dev up` in the project (it serves the edge). |
| `devenv-stack-not-loaded` | resolve, edge | 409 | The environment has no deployment of this stack manifest and no override for its alias. | Run `a4 dev up`: it loads every stack in `arete.lock`. |
| `devenv-program-release-mismatch` | resolve, Program Read, edge | 409 | The program's executable in the environment differs from the installed release, and no override is registered. The details name both executable identities (`installedExecutableIdentitySha256`, `liveExecutableIdentitySha256`). | Run `a4 dev up` with the program's local IDL to register it, or reset the environment. |
| `devenv-protocol-not-deployed` | resolve, edge | 409 | The environment has no active deployment of this protocol. | Run `a4 dev up` in the protocol's project. |
| `devenv-protocol-not-owned` | resolve, edge | 403 | The protocol belongs to another account; only your own protocols run in an environment. | Call it in production (unset `ARETE_ENVIRONMENT`). |
| `devenv-surface-unavailable` | resolve, edge, SDK | 503 | The environment can't serve this surface (not enabled for the account, or the installed SDK has no binding for it). | Named per case; never "use production". |
| `devenv-wallet-send-unsupported` | SDK | — | Dev mode, and the wallet adapter would send the transaction itself (its own RPC is mainnet) instead of signing only. | Use a sign-only wallet adapter, which submits through the client's transaction transport. |
| `environment-token-required` | dev services | 401 | A dev service received a token without an `env` claim. | Mint with `environment` (direct mode) or use the edge. |
| `environment-token-refused` | production services; dev services | 403 | A production service received an environment token, or a dev service received a token for another environment. | Use the token's own environment, or unset the switch for production. |
| `INVALID_CONFIG` | SDK | — | The switch, an edge URL or an environment reference is malformed. | Named per case. |

A dev environment is a fork of mainnet. A program it never changed has its
mainnet executable there, so Program Read verifies and decodes it with the
installed release. Some programs differ on every fork from the start: the
fork runtime ships its own builds of SPL Token and Token-2022. Arete checks
those builds against mainnet's (same account reads and writes) and serves
them as the installed release; any other changed program is refused with
`devenv-program-release-mismatch`.

## 7. Conformance

An SDK conforms when it passes every vector in
[`test-vectors/dev-environment-switch/v1/`](../../test-vectors/dev-environment-switch/v1/README.md)
(`switch.json`, `targets.json`, `refusals.json`) and, with the switch unset,
its existing production tests pass unchanged.
