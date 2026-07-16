# Ore Server Example

This example demonstrates how to run a Arete server for the Ore protocol.

## Running

```bash
cargo run
```

The server starts a WebSocket endpoint at `ws://localhost:8878` and an HTTP
read/health endpoint at `http://localhost:8081`.

Transaction relay routes are disabled by default. To enable the six fixed
`/transactions/v1/*` routes for local development, configure a dedicated RPC
URL and opt in explicitly:

```bash
ARETE_TRANSACTIONS_ENABLED=true \
ARETE_TRANSACTION_RPC_URL=https://your-solana-rpc.example \
cargo run
```

This example uses the server's explicit development `allow_all` behavior. A
public deployment should configure `SignedSessionAuthPlugin`; transaction
tokens need `transaction:inspect` and/or `transaction:send` independently.
The relay never signs transactions and does not retry `sendTransaction`.

Point the React example at it with:

```bash
VITE_ARETE_WS_URL=ws://localhost:8878 \
VITE_ARETE_HTTP_URL=http://localhost:8081 \
npm run dev
```

## Stack

This server uses the ore-stack from `../../stacks/ore`.
