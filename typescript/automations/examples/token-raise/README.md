# Automatic refunds for a token raise

When a raise fails, every contributor who opted in is refunded without
opening the app. `refund` is a participant step: it needs the
contributor's own signature, so this backend signs it for them with a key
they delegated, inside a consent they signed.

This is a pattern to adapt to your own protocol, not a published package.
It assumes a protocol shaped like a token raise: a `contribute` step, a
`refund` participant step, and a `raise.failed` event emitted when the raise
ends below its soft cap. `token-raise.ts` is a stand-in for that protocol's
generated client; replace it with your protocol's client and rename the
steps and event to match.

## Develop with `a4 dev`

`automations.ts` holds the handlers. In the protocol's project, point
`a4 dev up` at it:

```toml
# arete.toml
[dev.automations]
entry = "automations.ts"
```

```bash
npm install --save-dev @usearete/automations tsx
a4 dev up
```

`a4 dev up` deploys the protocol to your dev environment, a fork of
mainnet, generates its client, and runs the automations against that
deployment. It signs for the dev wallets, keeps jobs and consent under
`.arete/dev/automations/`, forwards the environment's events as signed
webhook deliveries, and restarts them when you save `automations.ts` or the
client is regenerated. Opt a dev wallet in and let a raise fail:

1. Create a raise with a deadline a few minutes away, leaving time to
   contribute (any minimum the create step enforces still applies), and a
   soft cap above what alice will contribute, so it fails.
2. Opt alice in and contribute before the deadline:

   ```bash
   a4 dev wallet alice                          # a funded dev wallet
   a4 dev consent alice --steps refund          # alice signs the consent
   a4 dev step contribute --instance <raise> --args '{"amount":"1000000"}' --wallet alice
   ```

3. Wait for the deadline to pass. The raise fails, `raise.failed` arrives
   and alice's refund runs:

   ```text
   automations │ completed: refund for <alice's address> on <raise>: completed
   ```

Don't use `a4 dev warp` to skip ahead: it moves the dev environment's chain
clock only, and the raise's deadline follows real time, so warping does not
end the raise.

To run the whole server below against the dev environment instead (the
consent routes too), use `forward = "http://localhost:8787/arete/webhook"`
and start `server.ts` from the protocol's project:

```bash
ARETE_SECRET_KEY=<secret key> APP_API_TOKEN=dev \
ARETE_WEBHOOK_SECRET="$(cat .arete/dev/automations/webhook-secret)" \
PROGRAMS='["<program id>"]' npx tsx server.ts
curl -X POST localhost:8787/dev/consent -H "authorization: Bearer dev" -d '{"wallet":"alice"}'
```

With the dev client it runs on `@usearete/automations/dev` only: dev
wallets sign (and Privy, for any other wallet, with `SIGNER=privy`), jobs
and consent are kept in `.arete/dev/token-raise-store.bin`, transactions
simulate on the fork, and `POST /dev/consent` signs alice's consent with
her dev wallet, as `a4 dev consent` does in entry mode. None of it runs
with the published client: each helper throws when the client targets
production.

## Run it in production

1. Install your protocol's client and point `token-raise.ts` at the
   generated module. `server.ts` and `automations.ts` import the protocol
   definition from it as `TOKEN_RAISE_PROTOCOL`, so re-export it under that
   name or rename the import:

   ```bash
   a4 install protocol <your-protocol>
   echo "export { YOUR_PROTOCOL as TOKEN_RAISE_PROTOCOL } from './generated/typescript/protocols/<your-protocol>/<your-protocol>.js';" > token-raise.ts
   ```

   Develop against a dev environment first: in the protocol's project,
   `a4 dev up` deploys it to a fork of mainnet and generates a client for
   that deployment, so the automations target `dev`. `a4 install --locked`
   puts the published client back, which targets `mainnet`.

2. Set the environment:

   | Variable | |
   |---|---|
   | `ARETE_SECRET_KEY` | A secret key for the deployment |
   | `ARETE_WEBHOOK_SECRET` | The signing secret of a webhook endpoint subscribed to `raise.failed`, pointing at `/arete/webhook` |
   | `DATABASE_URL` | Postgres for the store (a dev client uses a file under `.arete/dev/` instead) |
   | `RPC_URL` | Simulates each transaction (mainnet). With a dev client the fork is used instead, through `a4 dev up`'s local RPC |
   | `ALLOW_MAINNET` | `1` to let the policy sign on mainnet; without it only the dev environment is allowed |
   | `PROGRAMS` | JSON list of the protocol's program ids |
   | `APP_DOMAIN` | The host name users see in the consent message |
   | `APP_API_TOKEN` | A token your app's backend sends on the consent routes; in a real app, use your session auth and check the user owns the wallet |
   | `SIGNER` | `keypair` (with `DEV_WALLET`, a Solana CLI keypair file) or `privy` |
   | `PRIVY_APP_ID`, `PRIVY_APP_SECRET`, `PRIVY_AUTHORIZATION_KEY`, `PRIVY_WALLETS` | With `SIGNER=privy`: the app, its authorization key, and a JSON map of wallet address to Privy wallet id |

3. Migrate once, then start:

   ```bash
   npx tsx server.ts --migrate
   npx tsx server.ts
   ```

4. Opt a contributor in from your app:

   ```bash
   curl -X POST localhost:8787/consent/message -H "authorization: Bearer $APP_API_TOKEN" \
     -d '{"wallet":"<address>","expiresAt":1798761600000}'
   # sign `message` with the wallet (signMessage), then:
   curl -X POST localhost:8787/consent/grant -H "authorization: Bearer $APP_API_TOKEN" \
     -d '{"wallet":"<address>","expiresAt":1798761600000,"nonce":"<nonce>","issuedAt":<issuedAt>,"signature":"<base58>"}'
   ```

5. Let the raise fail: in the dev environment, create one with a short
   deadline and a soft cap it won't reach, contribute, and wait for the
   deadline (`a4 dev warp` moves the chain clock only and does not end it).
   The `raise.failed` webhook arrives, the handler finds the opted-in
   contributors of that raise, and each refund runs:

   ```text
   [completed] refund for <address>: completed
   ```

Opting out: `curl -X DELETE localhost:8787/consent/<address> -H "authorization: Bearer $APP_API_TOKEN"`.
Revoking needs no wallet signature, but it does need your app's authority:
anyone who could call it could stop another user's refunds.

## What it shows

- One handler fanning out over consenting wallets, filtered to the raise's
  participants.
- Consent limited to the network fee: a refund moves funds into the wallet,
  so only fees may leave it (`perExecution`/`perWindow` on `SOL`).
- A policy with a program allow-list and a small SOL cap.
- A Postgres store, migrated outside startup.
- The same code with a dev keypair or Privy.
