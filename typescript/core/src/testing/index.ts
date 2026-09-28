/**
 * `@usearete/sdk/testing`: supported helpers for testing applications built
 * on the Arete SDK without a network, a wallet or a chain.
 *
 * - {@link createWebSocketHarness}: a scripted WebSocket server for real clients.
 * - {@link createFrameHarness} and {@link frames}: the store engine and frame builders.
 * - {@link createFakeTransactionTransport}: a recording transaction relay.
 * - {@link createWalletFixture}: a recording wallet adapter with scripted outcomes.
 * - {@link createFetchStub}: a routed, recording `fetch`.
 * - {@link createTransactionOutcomeFixtures}: every transaction outcome.
 *
 * These helpers depend only on `@usearete/sdk` and the platform (`fetch`
 * types, `atob`); they work under any test runner.
 */
export {
  createFakeTransactionTransport,
  firstSignatureOfWireTransaction,
  type FakeTransactionCall,
  type FakeTransactionTransport,
  type FakeTransactionTransportOptions,
} from './transactions';
export {
  FIXTURE_WALLET_ADDRESS,
  createWalletFixture,
  type WalletFixture,
  type WalletFixtureCall,
  type WalletFixtureOptions,
  type WalletFixtureResponse,
} from './wallet';
export {
  FIXTURE_SIGNATURE,
  FIXTURE_SLOT,
  createTransactionOutcomeFixtures,
  transactionFailureError,
  transactionOutcomeFixture,
  type TransactionOutcomeFixtureName,
} from './outcomes';
export {
  createFrameHarness,
  frames,
  type FrameHarness,
  type FrameHarnessOptions,
  type FrameTarget,
} from './frames';
export {
  FakeWebSocket,
  createWebSocketHarness,
  type WebSocketHarness,
  type WebSocketHarnessOptions,
} from './websocket';
export {
  createFetchStub,
  jsonResponse,
  type FetchStub,
  type FetchStubOptions,
  type FetchStubReply,
  type FetchStubRequest,
  type FetchStubRoute,
} from './fetch';
