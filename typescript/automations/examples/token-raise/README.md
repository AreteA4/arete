# Automatic refunds for a token raise

When a raise fails, every contributor who opted in is refunded without
opening the app. `refund` is a participant step: it needs the
contributor's own signature, so this backend signs it for them with a key
they delegated, inside a consent they signed.

## Run it

1. Install the protocol and replace `token-raise.ts` with the generated
   module:

   ```bash
   a4 install protocol token-raise
   ```

2. Set the environment:

   | Variable | |
   |---|---|
   | `ARETE_SECRET_KEY` | A secret key for the deployment |
   | `ARETE_WEBHOOK_SECRET` | The signing secret of a webhook endpoint subscribed to `raise.failed`, pointing at `/arete/webhook` |
   | `DATABASE_URL` | Postgres for the store |
   | `RPC_URL`, `CLUSTER` | The cluster the deployment runs on (default devnet) |
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

5. Let the raise fail (on a dev environment, warp past its deadline). The
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
