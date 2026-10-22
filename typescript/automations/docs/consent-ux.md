# Consent UX

Users opt in to automations by signing a message with their wallet. That
signature is the record of what they agreed to, so the screen around it has
to say the same thing the message says.

## Before the signature

Show, in plain words, everything the message contains:

- **What will happen**, in your product's language: "We'll refund you
  automatically if this raise fails." Not "authorize step `refund`".
- **The limits**, in units people read: "Up to 50 USDC per buy, at most 200
  USDC in any 30 days." The message itself states base units; show the
  human amount next to it.
- **The settings they chose**: the amount, the day.
- **When it ends**: a date, not a duration.
- **How to stop it**: "Turn this off any time in Settings. No signature
  needed."

Then ask for the signature. The wallet shows the raw message. Because it is
written for people (see `consentMessage`), your screen and the wallet's
prompt should read as the same agreement.

## Rules

- **Never pre-tick or bundle consent.** Automations get their own screen
  and their own signature, separate from sign-in or a first deposit.
- **One grant per purpose.** Don't ask for broader steps or limits than the
  feature needs. Raising a limit later needs a new signature, which is the
  point.
- **Expire grants.** Choose an end date that matches the feature, for
  example the end of a raise. Grants scoped to an instance end on their own
  when it is terminal.
- **Make stopping easy and immediate.** Revoking needs no signature and
  stops jobs before they sign. Put it in the same place users found the
  feature.
- **Tell users what ran.** Use `onOutcome` to notify on every completed,
  refused and failed run, with the amount and a link to the transaction.
  `refused` (over a limit) and `rejected` (the protocol said no) are
  normal; say why.
- **Be honest about delegation.** If the user must add your app as a
  session signer in their embedded wallet, say what that allows. The
  consent message bounds what your backend does; the wallet provider's
  policy bounds what your key can do if your backend is ever compromised.
  Don't claim more than that.

## When `grant` warns

`consent.grant` returns warnings when the signer reports no delegation to
the wallet, or no provider policy bounding it. Treat a warning as a setup
step the user still has to finish, for example adding the session signer.
Don't start automations for that wallet until it is resolved.
