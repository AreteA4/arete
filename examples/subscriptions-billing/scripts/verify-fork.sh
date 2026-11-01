#!/usr/bin/env bash
# Drives the protocol on an `a4 dev` fork: every role's happy path and the
# rejections that keep the roles apart. Manual: it needs an Arete login with
# dev environment access (set ARETE_PROFILE to choose the a4 credential
# profile). Nothing here touches mainnet.
#
#   npm install && ./scripts/verify-fork.sh
#
# ARETE_DEV_ENV picks the environment; the default is the project's.
set -euo pipefail

cd "$(dirname "$0")/.."
export A4_NON_INTERACTIVE=1

MINT="${MINT:-EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v}" # USDC, an SPL Token mint
AMOUNT=1000000
PERIOD_HOURS=720
DEFAULT_PUBKEY=11111111111111111111111111111111
RUN="$(date +%s)"
FAILED=0

say() { printf '\n== %s\n' "$*"; }
pass() { printf 'ok    %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILED=1; }

pubkey() { a4 dev wallet "$1" --json 2>/dev/null | jq -r .pubkey; }

# step <step> <wallet> <instance or ""> <args json>: prints the step's JSON result.
step() {
  local instance=()
  if [[ -n "$3" ]]; then instance=(--instance "$3"); fi
  a4 dev step "$1" --wallet "$2" ${instance[@]+"${instance[@]}"} --args "$4" --json 2>&1 || true
}

expect_completed() { # <label> <step> <wallet> <instance> <args>
  local out
  out="$(step "$2" "$3" "$4" "$5")"
  if grep -q '"state":"completed"' <<<"$out"; then
    pass "$1"
  else
    fail "$1: $(tail -n 2 <<<"$out" | cut -c1-400)"
  fi
  LAST_RESULT="$out"
}

expect_rejected() { # <label> <code> <step> <wallet> <instance> <args>
  local out
  out="$(step "$3" "$4" "$5" "$6")"
  if grep -q "\"state\":\"completed\"" <<<"$out"; then
    fail "$1: the step completed"
  elif grep -q "$2" <<<"$out"; then
    pass "$1 ($2)"
  else
    fail "$1: expected $2, got $(tail -n 2 <<<"$out" | cut -c1-400)"
  fi
}

say "Environment"
a4 dev up --once --no-sdk --no-ui --port-base 0 2>&1 | grep -E 'a4 dev is ready|Deployed release|rror' | cut -c1-200

MERCHANT="merchant-$RUN" PULLER="puller-$RUN" SUBSCRIBER="subscriber-$RUN" STRANGER="stranger-$RUN"
for wallet in "$MERCHANT" "$PULLER" "$SUBSCRIBER" "$STRANGER"; do a4 dev fund "$wallet" 10 >/dev/null 2>&1; done
MERCHANT_KEY="$(pubkey "$MERCHANT")" PULLER_KEY="$(pubkey "$PULLER")" SUBSCRIBER_KEY="$(pubkey "$SUBSCRIBER")"
a4 dev fund "$SUBSCRIBER" --mint "$MINT" --amount $((AMOUNT * 5)) >/dev/null 2>&1

TOKEN_JSON="$(a4 dev token --json 2>/dev/null)"
RPC_URL="$(jq -r .rpcUrl <<<"$TOKEN_JSON")"
RPC_TOKEN="$(jq -r .token <<<"$TOKEN_JSON")"
export RPC_TOKEN
values() { npx tsx scripts/chain-values.ts "$RPC_URL" "$1" "$SUBSCRIBER_KEY"; }

say "createPlan"
CREATE_ARGS="$(jq -nc --arg id "$RUN" --arg mint "$MINT" --arg amount "$AMOUNT" --arg period "$PERIOD_HOURS" \
  --arg merchant "$MERCHANT_KEY" --arg puller "$PULLER_KEY" --arg zero "$DEFAULT_PUBKEY" '{
    planId: $id, mint: $mint, amount: $amount, periodHours: $period,
    planData: {
      planId: $id, mint: $mint,
      terms: { amount: $amount, periodHours: $period, createdAt: "0" },
      endTs: "0",
      destinations: [$merchant, $zero, $zero, $zero],
      pullers: [$puller, $zero, $zero, $zero],
      metadataUri: [range(128) | 0]
    }
  }')"
expect_completed "merchant creates the plan" createPlan "$MERCHANT" "" "$CREATE_ARGS"
INSTANCE="$(jq -r .instanceId <<<"$(tail -n 1 <<<"$LAST_RESULT")")"
expect_rejected "a second create for the same plan id fails" '"Custom":518' createPlan "$MERCHANT" "" "$CREATE_ARGS"

# An instance appears when the runtime has applied the create confirmation.
READY=""
for _ in $(seq 1 12); do
  if PLAN="$(a4 dev instance "$INSTANCE" --wallet "$MERCHANT" --json 2>/dev/null | jq -er .accounts.plan)"; then READY=1; break; fi
  sleep 5
done
if [[ -z "$READY" ]]; then
  fail "the instance never appeared; see \`a4 dev instance $INSTANCE --wallet $MERCHANT\`"
  exit 1
fi
pass "instance $INSTANCE, plan $PLAN"

say "Subscriber: authority and subscribe"
expect_completed "subscriber inits the authority" initAuthority "$SUBSCRIBER" "$INSTANCE" '{}'
expect_rejected "a second init is refused" authority_exists initAuthority "$SUBSCRIBER" "$INSTANCE" '{}'
CHAIN="$(values "$PLAN")"
SUBSCRIBE_ARGS="$(jq -c '{ subscribeData: {
  planId, planBump, expectedMint: .mint, expectedAmount: .amount, expectedPeriodHours: .periodHours,
  expectedCreatedAt: .createdAt, expectedSubscriptionAuthorityInitId: .authorityInitId
} }' <<<"$CHAIN")"
expect_completed "subscriber subscribes" subscribe "$SUBSCRIBER" "$INSTANCE" "$SUBSCRIBE_ARGS"

say "Collecting: who may"
COLLECT_ARGS="$(jq -nc --arg subscriber "$SUBSCRIBER_KEY" --arg destination "$MERCHANT_KEY" --arg amount "$AMOUNT" --arg mint "$MINT" '{
  subscriber: $subscriber, destination: $destination,
  transferData: { amount: $amount, delegator: $subscriber, mint: $mint }
}')"
expect_rejected "collect as the subscriber" not_merchant collect "$SUBSCRIBER" "$INSTANCE" "$COLLECT_ARGS"
expect_rejected "collect as a stranger" not_merchant collect "$STRANGER" "$INSTANCE" "$COLLECT_ARGS"
expect_rejected "collectAsPuller as a stranger" not_puller collectAsPuller "$STRANGER" "$INSTANCE" "$COLLECT_ARGS"
BAD_DESTINATION="$(jq -c --arg d "$PULLER_KEY" '.destination = $d' <<<"$COLLECT_ARGS")"
expect_rejected "collect to a destination off the plan" destination_not_allowed collect "$MERCHANT" "$INSTANCE" "$BAD_DESTINATION"
expect_completed "merchant collects" collect "$MERCHANT" "$INSTANCE" "$COLLECT_ARGS"
a4 dev warp "+$((PERIOD_HOURS + 1))h" >/dev/null 2>&1
expect_completed "puller collects the next period" collectAsPuller "$PULLER" "$INSTANCE" "$COLLECT_ARGS"

say "Cancel and resume"
expect_completed "subscriber cancels" cancel "$SUBSCRIBER" "$INSTANCE" '{}'
RESUME_ARGS="$(values "$PLAN" | jq -c '{ resumeData: { expectedExpiresAtTs: .expiresAtTs } }')"
expect_completed "subscriber resumes" resume "$SUBSCRIBER" "$INSTANCE" "$RESUME_ARGS"

say "updatePlan"
CHAIN="$(values "$PLAN")"
UPDATE_ARGS="$(jq -c '{ updatePlanData: {
  status: 1, endTs: .endTs, pullers, metadataUri,
  expectedCreatedAt: .createdAt, expectedEndTs: .endTs, expectedPullers: .pullers, expectedMetadataUri: .metadataUri
} }' <<<"$CHAIN")"
expect_rejected "updatePlan as the subscriber" not_merchant updatePlan "$SUBSCRIBER" "$INSTANCE" "$UPDATE_ARGS"
expect_completed "merchant updates the plan" updatePlan "$MERCHANT" "$INSTANCE" "$UPDATE_ARGS"

say "What each wallet can do"
for wallet in "$MERCHANT" "$PULLER" "$SUBSCRIBER" "$STRANGER"; do
  printf '%s: ' "$wallet"
  a4 protocol actions --dev --wallet "$wallet" --instance "$INSTANCE" --json 2>/dev/null |
    jq -c '[.instances[]?.steps[]? | select(.allowed) | .step]'
done

if [[ "$FAILED" -ne 0 ]]; then
  say "FAILED"
  exit 1
fi
say "All checks passed"
