# atrium-oauth spike findings (round-2 item 14)

**Question:** the identity plan assumes a 0.x crate (`atrium-oauth`) supports
the full server-side OAuth flow (handle to DID to PDS resolution, PAR, DPoP
P-256, PKCE S256, session persistence, token refresh) against ARBITRARY
member-chosen PDSes, not just bsky.social. Does it?

**Verdict: YES for the load-bearing part (resolution + PAR + authorize is
PDS-agnostic).** Recorded 2026-07-16.

## What was built

`src/lib.rs` is the exact one-file wrapper the stack decision mandates
(`atproto-stack-decisions.md`: "own a thin wrapper layer"): one type,
`WikiOAuth`, wrapping `atrium-oauth` 0.1.7 + `atrium-identity` 0.1.9 with a
public-client (localhost) metadata profile, atproto + transitional-generic
scopes, Cloudflare DoH for handle TXT resolution, the default PLC directory
for DID resolution, and in-memory state/session stores. It exposes
`begin_login(handle_or_pds) -> authorize URL`. This wrapper file is the same
shape the AppView ships.

## What was measured (live, `./target/debug/oauth-spike <targets>`)

`begin_login` runs handle resolution, DID resolution, PDS discovery,
protected-resource + authorization-server metadata resolution, and PAR (with
a fresh DPoP key and PKCE challenge), then returns the authorization URL.
Results:

| Input | Resolved authorization server | Outcome |
| - | - | - |
| `bsky.app` (handle) | `bsky.social` | OK |
| `atproto.com` (handle) | `bsky.social` | OK |
| `bnewbold.net` (handle) | `pds.robocracy.org` (INDEPENDENT) | OK |
| `https://bsky.social` (entryway) | `bsky.social` | OK |
| `https://pds.witchcraft.systems` (independent entryway) | itself | OK |

Two of the successes resolve to independent, non-Bluesky authorization
servers (`pds.robocracy.org`, `pds.witchcraft.systems`), which is the
PDS-agnostic proof: the crate PARs against whatever server the identity's PDS
advertises. (A bare hostname like `bsky.social` with no scheme is treated as a
handle and 404s, correctly: it is not an account. Pass a handle, a DID, or an
`https://` entryway URL.)

## Honest limits and what is NOT proven here

- **Token exchange + refresh not exercised end to end.** Completing the flow
  needs the member to visit the authorization URL in a browser and be
  redirected back with a code, then `OAuthClient::callback` exchanges it (this
  is where session persistence and refresh live). That step needs a human;
  it is proposed to the owner alongside the onboarding walkthrough (item 16),
  not run here. The crate exposes `callback` and a session store, so the API
  surface exists; only interactive confirmation is outstanding.
- **In-memory stores.** The spike uses `MemoryStateStore` / `MemorySessionStore`;
  the AppView needs durable stores (the state store spans the redirect, the
  session store holds the DPoP-bound tokens). That is a store-impl swap, not a
  flow change.
- **What this REPLACES.** A positive answer means `atrium-oauth` supersedes the
  ~28.8K hand-rolled `backend/src/oauth.rs` (plus `dpop.rs`, `pkce.rs`). A
  negative answer would have kept those as the permanent implementation. The
  wrapper here is the keeper artifact either way.
