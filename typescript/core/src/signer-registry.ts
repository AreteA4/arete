export interface SignerRegistry<TSigner = unknown> {
  register(address: string, signer: TSigner): void;
  unregister(address: string): boolean;
  get(address: string): TSigner | undefined;
  has(address: string): boolean;
  addresses(): readonly string[];
  values(): readonly TSigner[];
  entries(): readonly (readonly [string, TSigner])[];
  clear(): void;
}

export function createSignerRegistry<TSigner = unknown>(
  entries: Iterable<readonly [string, TSigner]> = []
): SignerRegistry<TSigner> {
  const signers = new Map<string, TSigner>();

  for (const [address, signer] of entries) {
    if (!address) {
      throw new Error('Signer registry addresses must not be empty');
    }
    signers.set(address, signer);
  }

  return {
    register(address, signer) {
      if (!address) {
        throw new Error('Signer registry addresses must not be empty');
      }
      signers.set(address, signer);
    },
    unregister(address) {
      return signers.delete(address);
    },
    get(address) {
      return signers.get(address);
    },
    has(address) {
      return signers.has(address);
    },
    addresses() {
      return [...signers.keys()];
    },
    values() {
      return [...signers.values()];
    },
    entries() {
      return [...signers.entries()];
    },
    clear() {
      signers.clear();
    },
  };
}
