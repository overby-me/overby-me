# Board custody — owner sign-off sheet

Six sub-calls on how the public bulletin board is published and mirrored
(`docs/ballot-board-custody.md`). The doc's analysis lands on **Org-published (B)
with signed inclusion receipts, a signed close-out digest, and ≥1 independent
mirror** — the only option this membership can actually use (per the DID audit
and onboarding reality), and the one that converts the risk it adds (censorship)
from silent to publicly provable. The recommended default is pre-filled for each;
ratify or amend.

| # | Question | Recommended default | Verdict |
|---|----------|---------------------|:------:|
| **1** | **Receipt signing key** — what signs inclusion receipts? | A **dedicated receipt keypair** (or the org's atproto DID key). NOT the poll's RSA issuer key — reusing the blind-signature key for plain signing mixes key usages. | ☐ default / ✏️ |
| **2** | **Publication latency** — when do entries hit the board? | **Micro-batched and shuffled** (breaks the per-entry cast-timing correlation a live firehose would leak, while keeping a near-live board). Alternatives: immediate (live but timing-correlating) or all-at-close (max timing privacy, no live board). | ☐ default / ✏️ |
| **3** | **Receipt scope** — what does a receipt commit to? | **Token + choices** (so content *substitution*, not just omission, is provable). | ☐ default / ✏️ |
| **4** | **Close-out digest** — is a signed digest mandatory before a tally is official? | **Yes, mandatory** — the signed close-out digest is the anti-censorship anchor; a tally without it is not official. | ☐ default / ✏️ |
| **5** | **Mirror commitment** — who runs the independent mirror, and is one running a precondition for the first binding poll? | Name **≥1 independent mirror operator**, and make a running mirror a **precondition for the first binding poll** (otherwise the censorship-is-provable property is theoretical). | ☐ default / ✏️ |
| **6** | **Board location** — org main repo, or a dedicated board account? | A **dedicated board account** (org-held keys) — keeps the board's records separate from the org's other content and gives it its own auditable history. | ☐ default / ✏️ |

## What this unblocks

Closing these six unblocks the **board-publish path** and the **inclusion-receipt
design** (`docs/ballot-verify-ux.md`), and the custody-dependent encoding
decisions — i.e. the public half of the ballot vertical. The private half
(eligibility / delegation / issuance) is already built and does not depend on
this (`crates/ballot-store`).

## What this does NOT unblock (read before assuming)

Closing custody does **not** by itself take `poll.json` / `ballotEntry.json` out
of PROVISIONAL. Both lexicons mark their crypto fields pending the `ballot-spec`
crate's pinned byte-level serialization (base64url-unpadded token/randomizer/
signature, DER SPKI issuer pubkey, plus a known-answer vector), which does not
exist yet. Pinning that encoding is a **separate, buildable-now scaffold task**,
independent of this owner call — the durable board already stores the entry body
as an opaque provisional blob precisely so this decision is not forced early.
