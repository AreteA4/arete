# Automatic refunds for a token raise

When a raise fails, every contributor who opted in is refunded without
opening the app. `refund` is a participant step: it needs the
contributor's own signature, so this backend signs it for them with a key
they delegated, inside a consent they signed.

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

```bash
a4 dev wallet alice                          # a funded dev wallet
a4 dev consent alice --steps refund          # alice signs the consent
a4 dev step contribute --instance <raise> --args '{"amount":"1000000"}' --wallet alice
a4 dev warp +2d                              # past the raise's deadline
```

```text
automations │ completed: refund for <alice's address> on <raise>: completed
```

To run the whole server below against the dev environment instead (the
consent routes too), use `forward = "http://localhost:8787/arete/webhook"`
and start `server.ts` with `ARETE_WEBHOOK_SECRET` set to the contents of
`.arete/dev/automations/webhook-secret`.

## Run it in production

1. Install the protocol and point `token-raise.ts` at the generated module
   (it exports `TOKEN_RAISE_PROTOCOL`, which `server.ts` imports):

   ```bash
   a4 install protocol token-raise
   echo "export * from './generated/typescript/protocols/token-raise/token-raise.js';" > token-raise.ts
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
   | `DATABASE_URL` | Postgres for the store |
   | `RPC_URL` | Simulates each transaction. Defaults to `a4 dev up`'s local RPC (`http://127.0.0.1:48700`) for a dev client; required for mainnet |
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

5. Let the raise fail (in the dev environment, `a4 dev warp` past its deadline). The
   `raise.failed` webhook arrives, the handler finds the opted-in
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
