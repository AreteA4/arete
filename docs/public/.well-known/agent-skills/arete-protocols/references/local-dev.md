# Development environments and self-serve deploys

`a4 dev` connects a local protocol project or Solana app to an Arete-hosted,
persistent mainnet fork. Source, wallets, the CLI and dev UI stay on your
machine. The environment's chain state and protocol runtime are hosted.
`a4 protocol deploy` is the separate production deployment workflow.

Use the released CLI's help and generated README as the source of truth:
`a4 dev --help`, `a4 dev up --help`, `a4 protocol deploy --help`.
Cloud access depends on the account's access and quotas. If the service is
unavailable or disabled, report that result; do not substitute production.

## Requirements and authorization

Use an Arete secret key from `a4 auth login` or `ARETE_API_KEY`. For a protocol
project, use CLI 0.42.1 or newer and Node.js 22.13+, then install the project's
npm dependencies. The cloud workflow does not require installing a validator
or local Arete runtime.

Starting a cloud environment creates or wakes an account resource and consumes
its allowance. Do it when the user asks to develop or test there. Development
SOL has no mainnet value. Never use production wallets or secrets for examples.

## CLI profiles and environment selection

A CLI credential profile chooses the account/key used for API access; a dev
wallet signs transactions on the fork. They are separate identities. Use the
profile appropriate to the caller, and keep it consistent across init, dev,
stream and deployment commands. For example, a human using stored human
credentials can pass `--profile human`; an agent should use its authorized
agent profile rather than borrowing the human profile to bypass a refusal.
Inspect `a4 auth status` and `a4 auth whoami` when access is unclear.

An environment need not be dedicated to one protocol. Reuse one with
`a4 dev up --env <existing-name>` when shared chain state is useful, or choose
another name for an isolated fork when quota allows. Projects sharing an
environment also share its account balances and lifecycle: sleeping or restoring
it affects the other projects. `--port-base 0` avoids local port conflicts but
does not create chain isolation. Use the global released `a4`; check its version
and help rather than introducing an operator-specific binary path.

## Start a project

```bash
a4 protocol init my-protocol
cd my-protocol
a4 init -y
npm install
npx arete-protocol check
a4 dev up
```

Init writes the project files and installs System Program builders. It refuses
to overwrite existing files. It also writes `automations/index.ts` and
`[dev.automations]`, so `a4 dev up` runs the project's automations from the
first session (see [automations](automations.md)). Commit source, generated modules and `arete.lock`;
keep `.arete/` ignored.

```bash
a4 dev up --env experiment          # choose or create a named environment
a4 dev up --no-deploy               # RPC and UI without protocol deployment
a4 dev up --port-base 48800         # another loopback port block
a4 dev status --json
a4 dev list --json                  # environments, usage and plan quotas
```

The CLI creates or wakes the environment, forwards its RPC and WebSocket on
loopback, builds and deploys the protocol when present, generates its client,
and watches for changes. A changed release gets a new deployment; reload code
that imported the previous generated definition. Failed builds leave the last
good deployment running. Use `deployment.*` expressions rather than hard-coded
ids or attestation keys.

`a4 dev down` stops local forwarding and the UI. It does not erase the cloud
fork. Environments sleep after inactivity and retain state for the account's
retention period. `a4 dev sleep` sleeps one explicitly; `a4 dev wake` wakes it.
Reset, restore and delete can discard work; get authorization for that effect.
Checkpoint storage and active environments count against the account's quotas.

## The app's client

The client is generated in `generated/<lang>/protocols/<alias>/`, where
`a4 install protocol` writes it. `a4 dev sdk --ts` (or `--rust`, `--python`)
regenerates it explicitly. Read its README for the real export and import path.
The definition carries the dev deployment id; do not edit generated files.

A backend uses the owning account's secret key and the API origin from
`a4 dev status --json`. The protocol API is not the loopback chain RPC. In
TypeScript, pass `keyClass: "secret"` when a key supplied through
`ARETE_API_KEY` must call secret steps. Browsers use an origin-bound publishable
key for public steps and an authenticated backend for secret steps.

Point wallets and chain clients at the RPC printed by `a4 dev up` (normally
`http://127.0.0.1:48700`). Keep the CLI running while they use it. Dev wallet
keys stay in `.arete/dev/wallets/`; only signed messages leave the machine.
`.arete/dev/session.json` and token-bearing stream URLs contain credentials:
never commit them or copy them into a browser bundle, log or chat.

Before shipping, deploy the protocol only when authorized, restore the published
client with `a4 install --locked`, and run `a4 install --check`. The latter fails
while a local-build override remains or dependencies differ from the lockfile.
Never force a production deployment id into a dev client or point the CLI at
an invented local protocol API.

## Drive and inspect the fork

```bash
a4 dev wallet creator
a4 dev step <step> --args '<json>' --wallet creator
a4 dev instance <instance-id> --wallet creator
a4 dev warp +2h
a4 dev fund <pubkey>
a4 dev scenario scenario.json --dry-run
a4 dev scenario scenario.json
a4 stream --dev --instance <instance-id>
a4 dev events
```

Use `warp` for deadlines and `scenario` for declared account overrides; inspect
the scenario before applying it. Use `a4 dev fund --help` for SOL/token funding
flags. Checkpoint, branch and restore are available through `a4 dev checkpoint`,
`a4 dev branch` and `a4 dev restore`; inspect their exact flags before changing
state. Untouched accounts follow mainnet; written accounts remain part of the
fork's state. Rebase refreshes untouched accounts, not the accounts you changed.

The MCP tools include `dev_status`, `dev_step`, `dev_instance`, `dev_actions`,
`dev_warp`, `dev_scenario` and `dev_events`. Hosted protocol tools accept
`dev: true` to use the project's dev deployment. See [agents.md](agents.md)
for the prepare/check/sign/submit flow and optional local signer.

## Validate a flow without prescribing its business logic

Use the actual spec and the wallet's available actions to choose steps and
arguments. Do not assume every protocol has create/pay/refund/close steps or
that every step is allowed at every state. A useful development pass is:

1. Inspect the deployment, wallet balances and available actions.
2. Start a stream or the dev UI before running a step, so its resulting rows
   can be compared with the execution result.
3. Run a permitted step with a named dev wallet. A create step has no instance
   id; later steps use the returned `instanceId` when their scope requires it.
4. Inspect the instance for the relevant wallet, the transaction result and
   balance changes. Wait for `completed`; a signature alone is not completion.
5. Exercise a meaningful rejection from the spec (wrong role, invalid argument,
   unmet state or terminal instance), and confirm that nothing was submitted.
6. Continue with another permitted step or create a fresh instance as the
   protocol requires. After editing the spec, watch for a successful redeploy
   and reload clients which imported the previous definition.

Use the UI, CLI or generated SDK according to the task; these are alternative
ways to drive the same flow, not a required sequence. CLI examples are:

```bash
a4 protocol actions --dev --wallet <dev-wallet> --json
a4 dev step <step> --wallet <dev-wallet> --args '<json>' --json
a4 dev instance <instance-id> --wallet <dev-wallet> --json
a4 stream --dev --instance <instance-id>
```

Inspect `state`, rejection codes and transaction errors in JSON output.
`expired`/`blockhash_expired` is a failed attempt, not a slow success. If the
submission outcome is ambiguous, inspect the existing intent and chain status
before starting another execution; never blindly resend. Current CLI versions
report execution phases on stderr while reserving stdout for the JSON result.

## What environment operations preserve

| Operation | Effect |
|---|---|
| `dev down` | Stops this project's local forwarding/UI; keeps the hosted fork. |
| `dev sleep` / `dev wake` | Stops/resumes the hosted environment with retained fork state. |
| `dev checkpoint` / `dev restore` | Saves/restores fork chain state; restore undoes later chain writes. |
| `dev rebase` | Refreshes untouched accounts from mainnet; retains fork-written/pinned accounts. |
| `dev reset` | Resets the fork, or the selected account with `--account`; inspect its flags and scope first. |
| `dev delete` | Deletes the environment and its retained state/checkpoints. |

A checkpoint does not snapshot the API's protocol instance, intent or execution
records, nor local CLI history. Restoring chain state does not reopen a terminal
recorded instance or undo a submitted API execution record. Check the current
instance/actions and use a fresh instance where appropriate. Restore is useful
for chain-state experiments, not as a promise of a whole-platform rollback.

The prepared transaction includes any applicable platform fee on a cloud fork.
Unknown fee classification does not block otherwise valid execution. See
[fees](authoring.md#fees) and [execution receipts](using.md#execution-receipts).

## Deploying your own protocol

Deploying is an external mutation: do it only when the user asked for it.

```bash
a4 protocol deploy --dry-run             # builds; files, expected release hash, slug, create vs reuse, programs to push
a4 protocol deploy --resource launchDefaults=launch-defaults.json
a4 protocol deploy --no-activate --json
```

It needs a secret key (publishable keys are refused). It:

1. **Builds** with `arete-protocol build`. `--no-build` uploads the existing
   `.arete/protocol/` output instead and refuses it, naming the changed
   files, when a source changed since that build.
2. **Pushes programs installed from a local IDL.** Arete only checks
   templates against, compiles stacks from and packages programs it has, so
   each `source = { idl = … }` program is registered as your private program
   (alias `<slug>-<alias>`) unless your account or the registry already has
   that exact ProgramSpec. It waits for the admission
   (`waiting: program <alias> admission queued`, bounded by `--wait-secs`),
   records the Program Release in `arete.lock` and rebuilds. An IDL that no
   longer matches `arete.lock` is refused: run `a4 install` first.
   `--dry-run` only lists these programs.
3. Uploads only `protocol-spec.json` (plus `handlers.mjs` when a step has a
   handler); the server derives the release (`npx arete-protocol build
   --server-release` predicts its hash; deploy warns if they differ).
4. Reuses or creates a private deployment. A new one needs a value for each
   declared resource (a JSON file path or an inline TOML value under
   `[protocol.resources]`). Arete sets the fees; there is no fee policy to
   write. `[protocol].creator_fee_wallet` (or `--creator-fee-wallet`)
   optionally names the wallet for the creator's share of Arete's fee.
   Defaults live in `[protocol]` of `arete.toml` (`access = "private"` is
   the only accepted value).
5. Activates it, unless `--no-activate`.
6. Updates the project's own client to the new deployment (below).

| Protocol | Outcome (`status` in `--json`) |
|---|---|
| Template-only | `active`: a private deployment at trust tier `unreviewed`. Activation waits (`waiting: …`, up to `--wait-secs`) while the server prepares the deployment and its stack. If the wait runs out, the deployment is not active yet and `pending` says why; run deploy again to finish. |
| Has handlers | `review-required`: the deployment exists, but handler code runs only after Arete reviews the release. Share the printed release hash and deployment id; Arete reviews and activates it. Owners never activate handler code. |
| Public | Not self-serve. Every self-serve deployment is private. |

An unreviewed deployment runs no author code, is visible only to your
account, and is not a reviewed or public listing. Don't present it as either.

### The registry package

Activation publishes an **owner-private** registry package of the protocol
(slug and spec version), reported as `registryPackage`:

| `status` | Meaning | Do |
|---|---|---|
| `published` | `Published the private registry package <slug>@<version>` | nothing |
| `existing` | already published for this release and deployment (`(already published)`) | nothing |
| `skipped` | nothing published, with `code` and `message`; the deployment is still active | `package-version-immutable`: that version already pins other content, so bump `version` in `protocol.ts` and deploy again. Otherwise fix what `message` names and deploy again. |

Then:

- **The project's own client.** If `arete.toml` installs this protocol
  (`a4 dev up` adds it), deploy moves the dependency to `^<deployed
  version>` and reinstalls the published package; the client now calls the
  production deployment. While no package is available it generates the
  client from the deployed build and keeps `local_build`. A project with no
  client gets a hint: `a4 install protocol <slug> --ts`.
- **Other projects of the same account** install it like a stack, with no
  deployment flag: `a4 install protocol <slug> --ts` (or `--rust`,
  `--python`). Other accounts can't resolve an owner-private package.
- **CI.** `a4 install --check` changes nothing and fails while any protocol
  client is a local build from `a4 dev` or isn't the release `arete.lock`
  pins. `a4 install --locked` restores the published clients. Both need the
  registry that published the package.

## Troubleshooting

- Service disabled or access denied: report the account/service result and keep
  production separate. Repeated retries do not grant access.
- RPC unavailable: check `a4 dev status`, the selected environment and whether
  `a4 dev up` is still forwarding. Environment tokens expire and are renewed
  by the CLI; do not hard-code them.
- Build or activation failure: read the diagnostics in the CLI or dev UI.
  Verify installed program modules and `arete.lock` before rebuilding.
- Wrong environment: inspect `a4 dev status --json` and `ARETE_DEV_ENV` before
  another mutation; use `a4 dev use --help` to select the intended environment.
- Missing generated client: run `a4 dev sdk` for the intended language and read
  its README. Reload the app after the deployment changes.

See the public [cloud dev guide](https://docs.arete.run/cli/local-development/)
and [application guide](https://docs.arete.run/development/application/) for
current commands and connection details.
