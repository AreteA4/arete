# Examples

Usage examples for Arete stacks.

## React

```bash
cd ../typescript/core && npm ci && npm run build
cd ../react && npm ci && npm install ../core --no-save --package-lock=false && npm run build
cd ../adapters/web3js && npm ci && npm install ../../core --no-save --package-lock=false && npm run build
cd ../../../examples/ore-react
npm install && npm run dev
```

The hosted ORE stack requires `VITE_ARETE_PUBLISHABLE_KEY` in `.env.local`. See `ore-react/README.md` for setup details.

## Rust

```bash
cd ore-rust
cargo run
```

## Server

```bash
cd ore-server
cargo run
```

## TypeScript (CLI)

```bash
cd ore-typescript
npm install && npm start
```
