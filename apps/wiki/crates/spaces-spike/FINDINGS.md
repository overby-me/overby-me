# spaces-spike: findings

Question: does the atproto spaces alpha (proposal 0016, "permissioned data") do
what a redesign of the wiki on it would lean on, and can the part a syncer must
get exactly right be done from Rust? Asked of the real thing, on 2026-09-20:
`ghcr.io/bluesky-social/atproto:pds-spaces-alpha` (PDS 0.5.32) in a container on
this machine, registering its made-up accounts with `crates/fake-plc`. The
redesign is `docs/atproto-spaces-redesign.md`.

## What held

- **A space under an organization's account.** `com.atproto.simplespace.createSpace`
  as the account `wikiorg.test`, type `com.example.wiki.context`, a key of our
  choosing: `at://{org}/space/com.example.wiki.context/c-spike1`. The account
  writes records into its own repo in the space (`com.atproto.space.putRecord`).
- **The credential flow.** `getDelegationToken` on the user's PDS (ES256K, `typ`
  `atproto-space-delegation+jwt`, 60 seconds, `aud` the authority's
  `#atproto_space_host`), exchanged at `getSpaceCredential` with a DPoP proof for
  a credential of two hours bound to the proof's key (`cnf.jkt`). Reads then
  take `Authorization: DPoP <credential>` and a proof with `ath`. The credential
  as a bare bearer is refused. A proof is an ES256 JWT over `htm`, `htu`, `iat`,
  `jti`, with no server nonce.
- **The managing app decides who gets in.** With `readPolicy` and `writePolicy`
  set to `managing-app`, the PDS asked a stand-in of ours
  `com.atproto.simplespace.checkUserAccess?space=..&user=..&access=read|write`,
  under a service token (`iss` the authority, `aud` our `did#fragment`, `lxm` the
  method). Answering from a rule of ours gave one account a credential and
  refused another with `UserNotAuthorized`. No member list on the PDS: the
  AppView's roster can be the only one.
- **Write notifications.** After `registerNotify` (good for 24 hours) a member's
  write arrived as `com.atproto.space.notifyWrite {space, repo, rev, hash}` under
  the same kind of service token. An outsider's write was asked about
  (`access=write`), refused, and not forwarded.
- **Sync.** `listRepos` is the writer set with each repo's `rev` and `hash`.
  `listRepoOps` returns the log with values inlined and the signed commit at its
  end; a record edited or deleted later appears as an operation without its
  stale value. `getRepo` serves a CAR, `getLatestCommit` a commit with a fresh
  nonce each time.
- **Files.** A blob uploaded with the ordinary `com.atproto.repo.uploadBlob` and
  named by a record in a space is served by `com.atproto.space.getBlob` to a
  credential, and NOT by the public `com.atproto.sync.getBlob`.
- **The commit, from Rust** (`src/lib.rs`, tests over a fixture the PDS served).
  The set hash is LtHash as the proposal has it: `{collection}/{rkey}/{cid}`
  through BLAKE3's XOF to 2048 bytes, 1024 little-endian `u16` lanes added or
  subtracted with wraparound, and the commit's `hash` the SHA-256 of the state.
  Listing the records and following the log (an edit is a removal of `prev` and
  an addition) arrive at the same hash. The signature is ECDSA over SHA-256 of
  the context (the tag `atproto-space-v1`, then space, author, revision and
  nonce, each behind a big-endian `u16` length), by the author's `#atproto` key,
  secp256k1 on this PDS. The MAC is HMAC-SHA256 of the hash under
  `HKDF-Expand(ikm, context, 32)`, which for 32 bytes is one HMAC block. A
  commit moved to another space fails its signature; another hash fails the MAC.

## What to build on knowing

- **Nobody is stopped from writing.** An account that is no member wrote into
  its own repo for the space, and `listRepoOps` served that repo to a credential
  that named it. The write policy decides only who is in the writer set and
  whose notifications are forwarded. Whatever syncs a space decides what counts,
  record by record: the wiki's rules on who may make what where have to run at
  ingest, as they run on its own write path today.
- **Reading is all of a space or none of it.** Another account's repo answered
  `RepoNotFound` to a session and everything to a credential. Nothing narrower
  exists, so nothing narrower than "every member of the context" can live in a
  context's space.
- **Files over 5 MB are refused by a PDS as it comes.** 4 MB went in; 6 MB broke
  the connection (`PDS_BLOB_UPLOAD_LIMIT` defaults to 5242880). The interim
  holds files up to 22.7 MB. On a PDS the organization runs this is a setting;
  on a member's it is not.
- **Password sessions work for every space method on this build.** The proposal
  speaks of OAuth scopes (`space:<type>?...`); the alpha did not ask for them of
  a `createSession` token. Not to be relied on.
- **`org` is a reserved handle.** Found by wanting it.

Found later the same day, by the AppView's mirror (`crates/appview/src/spaces.rs`)
and by hand against the same build:

- **No fractions.** A record with `0.5` in it is an `InvalidRequest` ("Expected
  one of null, boolean, integer, string, cid, bytes, array or object"). `1.0` and
  `1e3` are read as the integers they are. An integer past 2^53 is a 500. An
  array where a lexicon would want an object is taken, as is `null`: a record of
  a type the PDS does not know is not validated (`validationStatus: unknown`).
  So the mapping carries such numbers as `wiki.radikal.spaceDefs#number`.
- **A request past about 1 MB is refused** (`PayloadTooLargeError`, 413): 900 KB
  of record went in, 1 MB did not. A page that large cannot be one record.
- **A deleted space takes writes.** After `deleteSpace`, `putRecord` into the
  same space answered 200 with a CID, `deleteRecord` 200, and a second
  `deleteSpace` 200. `createSpace` on a space that exists is `SpaceAlreadyExists`,
  and one deleted can be made again. `getSpace` is what says whether a space is
  there (`SpaceNotFound`) and how it is set up, so the mirror asks it on every
  sweep, and `updateSpace` takes back one whose policy was changed.
- **The owner is not asked about.** The organization got a credential for its
  own space while the managing app it named could not be resolved at all.
- **Admitting applications by name works, and needs no confidential client.**
  A space made with `appAccess: #allowList` refuses a credential to a session
  that comes with no attestation (`AppNotAuthorized`), the authority's own
  included: the application is checked before the user, from the space's setup
  alone. An attestation is a JWT of type `atproto-client-attestation+jwt`, with
  `iss` and `sub` the `client_id`, `aud` `{authority}#atproto_space_host`, a
  `jti`, good for a minute and once, signed by a key named by `kid`. The host
  fetches the `client_id`, reads it as OAuth client metadata and takes the key
  from its `jwks` or `jwks_uri`. It does not ask that the client be
  confidential: `token_endpoint_auth_method: none` with a `jwks_uri` passes here
  and in the OAuth provider's own validation. It fetches by NAME only (a
  `client_id` on an IP address is refused even in dev mode), and over plain
  HTTP only with SSRF protection off, which is how the tests here reach an
  AppView on `localhost`.
- **A deleted space says so to whoever renews.** `getSpaceCredential` answers
  `SpaceDeleted` for one, which is what a syncer that missed
  `notifySpaceDeleted` learns from. (Read in the PDS's source, not yet seen.)
- **The managing app, for real.** With the AppView itself as the managing app,
  a member got a credential through her own session and read the organization's
  repo; who the roster did not name was `UserNotAuthorized`; anyone got in to a
  context that is open.

## What was not asked

OAuth `space:` scopes and the consent screen; `notifySpaceDeleted` as the PDS sends it (the AppView
answers it, and nothing registered to be told); a member on
another PDS than the authority's (every account here shared one); account
migration; how long a PDS keeps its operation log. The alpha promises breaking
changes weekly, so all of the above is as of the date at the top.
