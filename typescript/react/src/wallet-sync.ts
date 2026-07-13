import type { ConnectedArete, ConnectionState, StackDefinition, WalletAdapter } from '@usearete/sdk';

import type { ZustandAdapter } from './zustand-adapter';

type AnyClient = ConnectedArete<StackDefinition>;

export interface WalletAwareClientEntry {
  client: Pick<AnyClient, 'setWallet'>;
}

export function syncClientWallets(
  entries: Iterable<WalletAwareClientEntry>,
  wallet: WalletAdapter | undefined
): void {
  for (const entry of entries) {
    entry.client.setWallet(wallet);
  }
}

export function initializeConnectedClient(
  client: Pick<AnyClient, 'setWallet' | 'onConnectionStateChange' | 'connectionState'>,
  adapter: Pick<ZustandAdapter, 'setConnectionState'>,
  wallet: WalletAdapter | undefined
): void {
  client.setWallet(wallet);
  client.onConnectionStateChange((state: ConnectionState) => {
    adapter.setConnectionState(state);
  });
  adapter.setConnectionState(client.connectionState);
}
