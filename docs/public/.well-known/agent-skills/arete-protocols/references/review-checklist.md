# Review bundle checklist

A protocol with handlers needs Arete's review before it can be activated:
`a4 protocol deploy` stops at `review-required` and prints the release hash
and deployment id to share. Arete reviews the **server-derived release hash**
(`npx arete-protocol build --server-release` prints it before you upload)
and activates the deployment. Public deployments are Arete's decision too.
Walk this list before asking for a review.

Template-only protocols skip review and deploy privately at trust tier
`unreviewed`; still walk the Spec and Unreviewed sections before deploying.

## Identity

- [ ] Two clean builds produced identical `protocol-release.json` bytes.
- [ ] The receipt's outputs match the spec, bundle and stack-source hashes.
- [ ] Every program comes from an installed module (`a4 install program`,
      registry or local IDL) or `builtins(...)`, `arete-protocol build`
      passed its `arete.lock` check, and each program id is the program the
      author intends (checked against the catalog, not memory).
- [ ] A program installed from a local IDL is one the author means to
      register as their own private program: deploy pushes it, and
      `arete.lock` records the Program Release it became.
- [ ] `a4 install --check` passes: committed clients are published
      releases, not local builds from `a4 dev`.

## Spec

- [ ] Every step's authority, access and signer classes are what the flow
      needs; `create` is `secret` unless instance creation by anyone is
      deliberate.
- [ ] Role steps carry `wallet == instance.roles.<role>`.
- [ ] Keeper steps need no signer beyond keeper, ephemeral and attestation,
      and cannot move a participant's funds.
- [ ] Admin steps propose to a multisig read from a resource
      (`resources.<name>.multisig`), never from the instance, args, params,
      the wallet or a constant; no step creates a multisig.
- [ ] Every vault an admin step's instructions use names the step's
      `vaultIndex` (vault 0 when it has none).
- [ ] The docs tell the team to attach the multisig on the dashboard
      (https://app.arete.run/dashboard/squads) or add the proposer in the
      Squads app with Initiate only (mask `1`), and to put its address in the
      deployment config. The proposer never has Vote or Execute and is never
      counted toward the threshold.
- [ ] Platform keys come from `deployment.id`, `deployment.attestation` and
      `deployment.squadsProposer`, never hard-coded pubkeys or ids (they
      change per deployment and per integrator account).
- [ ] Guards cover deadlines, caps and states; codes are stable.
- [ ] Derived-state rules have `at` times where they depend on `now`.
- [ ] Fee bases name `u64` amount arguments; no handler emits fees or memos.
- [ ] Events are emitted only with ids and states.

## Unreviewed tier (self-serve)

- [ ] The release has no handler bundle; a handler means Arete's review.
- [ ] The deployment is private; nothing claims it is public or reviewed.
- [ ] Every template targets a program Arete has (a registry install, or a
      local IDL that deploy pushes), so the server's template check can
      pass.
- [ ] The flow was exercised under `a4 dev` (create, happy path, every guard
      rejection, deadlines via `a4 dev warp`), and the app ran against the
      dev environment with its generated client; published dependencies were
      restored and checked before shipping.

## Handler bundle (only if a step has one)

- [ ] The bundle is one ESM file; read it, not only the source.
- [ ] Data comes only from `ctx` and the input; no network, filesystem,
      clock, randomness, environment, `eval` or dynamic import.
- [ ] Instructions are built with generated program builders.
- [ ] Handler tests replay recorded provider snapshots.
- [ ] Each handler step says why a template could not express it.

## Generated stack

- [ ] The stack loads under `a4 dev up` without diagnostics (or, when the
      environment cannot decode a program, the refusal is understood);
      inspect generated build artifacts and the CLI diagnostics.
- [ ] Each `stack.activity` rule names the instruction account that
      identifies the instance, and every rule for one instruction names the
      same accounts and amount argument.
- [ ] Steps whose bypass activity matters use an instance-owned account
      (bound from an ephemeral key) as `instanceAccount`; direct counts are
      only matched there.
- [ ] `account`-visibility protocols scope every entity.
