import { defineProgramExtensions } from '@usearete/sdk/program';

import {
  SUBSCRIPTIONS,
  type FixedDelegation,
  type RecurringDelegation,
} from './subscriptions-core';

export type Address = string;

export const SUBSCRIPTIONS_PROGRAM_ADDRESS = SUBSCRIPTIONS.programId as Address;

/** @title Event authority address */
export function getEventAuthorityPda(): Address {
  return SUBSCRIPTIONS.addresses.eventAuthority.deriveSync({});
}

/** @title Subscription authority address */
export function getSubscriptionAuthorityPda(input: {
  user: Address;
  tokenMint: Address;
}): Address {
  return SUBSCRIPTIONS.addresses.subscriptionAuthority.deriveSync({
    args: { user: input.user, tokenMint: input.tokenMint },
  });
}

/** @title Merchant plan address */
export function getPlanPda(input: { owner: Address; planId: bigint }): Address {
  return SUBSCRIPTIONS.addresses.plan.deriveSync({
    args: { owner: input.owner, planId: input.planId },
  });
}

/**
 * Derives the delegation address shared by fixed and recurring delegation
 * records. Both instruction families use the same documented seed tuple and
 * are distinguished by the account data discriminator.
 *
 * @title Delegation address
 * @concept token-delegation
 */
export function getDelegationPda(input: {
  subscriptionAuthority: Address;
  delegator: Address;
  delegatee: Address;
  nonce: bigint;
}): Address {
  return SUBSCRIPTIONS.addresses.fixedDelegation.deriveSync({ args: input });
}

/** @title Fixed delegation address */
export const getFixedDelegationPda = getDelegationPda;

/** @title Recurring delegation address */
export const getRecurringDelegationPda = getDelegationPda;

/** @title Plan subscription address */
export function getSubscriptionPda(input: {
  planPda: Address;
  subscriber: Address;
}): Address {
  return SUBSCRIPTIONS.addresses.subscriptionDelegation.deriveSync({
    args: { planPda: input.planPda, subscriber: input.subscriber },
  });
}

/**
 * Reports whether a delegation is past its on-chain Unix expiry timestamp,
 * matching the program's spend gate: an `expiryTs` of 0 never expires, and a
 * finite expiry is still spendable during its own second (`nowTs > expiryTs`).
 *
 * @title Delegation expiry check
 * @concept recurring-payments
 */
export function isDelegationExpired(expiryTs: bigint, nowTs: bigint): boolean {
  return expiryTs !== 0n && nowTs > expiryTs;
}

/**
 * Returns the raw amount still available to a fixed delegation. It is zero
 * once the delegation expires; otherwise the account's remaining `amount` is
 * already the authoritative allowance.
 *
 * @title Fixed delegation remaining amount
 * @concept token-delegation
 */
export function getFixedDelegationRemainingAmount(
  delegation: FixedDelegation,
  nowTs: bigint,
): bigint {
  return isDelegationExpired(delegation.expiryTs, nowTs) ? 0n : delegation.amount;
}

/**
 * Returns the unused raw allowance in the recurring delegation's current
 * accounting period. This deliberately does not advance the period locally;
 * the on-chain account remains authoritative about rollover.
 *
 * @title Current recurring allowance
 * @concept recurring-payments
 */
export function getRecurringDelegationRemainingAmount(
  delegation: RecurringDelegation,
  nowTs: bigint,
): bigint {
  // A pull before the recorded period starts fails on-chain
  // (delegationNotStarted), so nothing is available yet.
  if (isDelegationExpired(delegation.expiryTs, nowTs) || nowTs < delegation.currentPeriodStartTs) {
    return 0n;
  }
  const remaining = delegation.amountPerPeriod - delegation.amountPulledInPeriod;
  return remaining > 0n ? remaining : 0n;
}

export const subscriptionsDevex = {
  SUBSCRIPTIONS_PROGRAM_ADDRESS,
  getEventAuthorityPda,
  getSubscriptionAuthorityPda,
  getPlanPda,
  getDelegationPda,
  getFixedDelegationPda,
  getRecurringDelegationPda,
  getSubscriptionPda,
  isDelegationExpired,
  getFixedDelegationRemainingAmount,
  getRecurringDelegationRemainingAmount,
} as const;

const programExtensions = defineProgramExtensions<typeof SUBSCRIPTIONS>()({
  constants: {
    program: SUBSCRIPTIONS_PROGRAM_ADDRESS,
  },
  addresses: {
    eventAuthority: getEventAuthorityPda,
    subscriptionAuthority: getSubscriptionAuthorityPda,
    plan: getPlanPda,
    delegation: getDelegationPda,
    fixedDelegation: getFixedDelegationPda,
    recurringDelegation: getRecurringDelegationPda,
    subscription: getSubscriptionPda,
  },
  math: {
    isExpired: isDelegationExpired,
    fixedRemainingAmount: getFixedDelegationRemainingAmount,
    recurringRemainingAmount: getRecurringDelegationRemainingAmount,
  },
});

export default programExtensions;
