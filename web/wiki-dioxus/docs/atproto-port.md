# atproto port — design notes

The current `web/wiki-dioxus` (Dioxus/WASM frontend + axum backend on top of
nhost/Hasura/Postgres) is an **interim** step. The plan is to **fully replace
Hasura + nhost + Postgres with a custom Rust backend + a new database**, built on
[atproto](https://atproto.com). This document collects the architecture thinking
so the rewrite starts from a clear picture rather than a blank page.

> Status: pre-work notes, not a committed spec. The pivotal decisions in §7 are
> deliberately left open.

## 1. Goal & framing

- **Full replacement**, not augmentation: the whole current stack (Hasura
  row-level security, computed fields, nhost auth, the current Postgres schema,
  the Dioxus frontend eventually) is interim/throwaway. Do **not** invest in
  optimising it beyond keeping it correct and usable while it runs real
  assemblies.
- The **custom backend is the atproto AppView + org-authority service** — an
  evolution of today's axum backend's role, but now the *whole* backend rather
  than an admin-secret sidecar next to Hasura.

## 2. What's throwaway vs. what carries over

**Throwaway** (Hasura/nhost specifics): RLS permission model, the per-row
computed fields (`get_index`, `is_owner`, `is_context_owner`), the GraphQL
surface, nhost auth, the physical Postgres schema, the `DATA_VERSION` global
invalidation, the per-query WebSocket subscriptions.

**Carries over** (the ideas, not the code):

- The **data-model concepts** — a hierarchical node tree with typed kinds
  (`wiki/*`, `vote/*`, `speak/*`), members-by-context, polls/ballots, comments.
- The **identity binding pattern** — invite-by-email (fixed Excel roster) →
  claim-token → durable user binding (`members.node_id` today).
- The **security thinking** — anonymity mediation for secret ballots, membership
  authority, SSRF-safe outbound, no-leak errors. The *reasoning* transfers even
  though the Hasura code does not (see `docs/` security notes / memory).

## 3. Five structural decisions

1. **Identity pivots to the DID.** Today `members.node_id` is the durable user
   binding and email is only the invite address. The rewrite promotes the
   **DID** to the primary identity of a person. Keep invite-by-email (the roster
   constraint doesn't change) but bind the claim-token flow to a DID. The current
   backend already stores a linked DID + encrypted atproto session in
   `user_providers`.
2. **The EAV node tree becomes typed lexicons.** The ~20 `mimeId` kinds are an
   informal schema living in `data` jsonb blobs. Author one **lexicon** per
   meaningful kind (`com.example.wiki.*` / your NSID). Content is already portable
   Slate JSON, so *content* survives; the parent/context *container* model is what
   becomes records + references.
3. **Permissions require a trusted service — atproto gives nothing here.** Authz
   today is server-enforced (Hasura RLS); atproto is "everyone owns their own
   repo, no central authority." An assembly tool is org-scoped and
   authority-bound, so the custom backend **must** own membership, roles,
   permissions, and canonical group state. Membership/roles are records the *org
   service* signs, not member self-assertions.
4. **Secret ballots + ephemeral state stay server-mediated.** atproto records are
   DID-signed, so an anonymous ballot-as-record is a contradiction; anonymous
   voting needs a trusted tallying service. Ephemeral coordination (the `active`
   projector node, `screenComments`) is coordination state, not content — it
   lives in the backend, not in durable records.
5. **Unify local↔remote state behind a real sync layer.** Today's `DATA_VERSION`
   bump (invalidation) and per-query `use_live` WebSockets (change notification)
   are two triggers feeding a cacheless refetch — a degenerate hand-rolled sync
   engine. Their hard sub-problems (invalidation scope, subscription scope) are
   the *same* question: which local state a remote change affects. Solve it once
   with a proper sync layer. atproto is built for this: the firehose/Jetstream is
   the single change-feed, an AppView is a server-side materialised cache.

## 4. The source-of-truth split (core design decision)

In an atproto assembly tool, state divides in two:

- **User-owned records** live in user repos (signed by their DID): their comment,
  their profile, arguably their *non-secret* vote. The firehose streams these;
  the backend's new DB **materialises** them for querying.
- **Org-authoritative state** has no home in any user repo — the membership
  roster + roles + permissions, the canonical agenda, the **official tally**, the
  projector/`active` state, and the secret-ballot dedup + anonymous ballots. This
  lives only in the new DB, owned by the org's service.

So the new DB is simultaneously a **materialised view of the firehose** and the
**authoritative store** for shared state.

### The privacy pivot (do not skip this)

**atproto records are PUBLIC** — everything in a repo is readable via the
firehose. For a political youth org, internal deliberation and secret ballots
**cannot** be atproto records. Realistically:

> atproto ≈ **identity (DID) + optional public artifacts** (public statements,
> profiles). The actual assembly machinery — debate, roster, secret votes, the
> official tally — stays in the **private backend DB**.

But visibility is **per-item, not global**. If the scope extends to a social
platform (feed, groups, events) where content can *optionally* be public, then
atproto is a first-class substrate for the public half — public posts/groups/
events become user- or org-owned records (a real, federatable platform), while
private content and the always-private machinery (secret ballots, roster,
authoritative state) stay in the DB. So it's a **public/private hybrid**: the DB
is the source of truth and public items are mirrored out as records. See
[`atproto-domain-model.md`](./atproto-domain-model.md) for the per-entity split,
lexicons, and the visibility model.

## 5. Tech stack

Researched against current (2026-07) sources for the expanded scope (general
social — groups/events/feeds — plus AI = vector/semantic search).

### Database — leaning SurrealDB (project is not correctness-critical)

- **SurrealDB** (pure Rust; one binary): document + first-class **graph** +
  **vector** search + **realtime LIVE queries**. For a non-critical, self-hosted,
  pure-Rust project it's attractive as the **primary store** because it collapses
  primary store + social graph + AI vectors + realtime + **sync transport** into
  one thing that fits the Nix/rustls/minimal-deps ethos.
  - **The prize: LIVE queries are the sync layer** from §3.5 — you subscribe to a
    query and the DB pushes changes to the client, instead of building
    `LISTEN/NOTIFY → WS → reactive cache` by hand.
  - **What you accept:** the beta tax (bugs, breaking upgrades, younger ecosystem
    — mature *with* the project); LIVE queries are single-node (fine at this
    scale); transaction isolation is not independently verified (no Jepsen).
  - **De-risk the vote path independent of DB guarantees:** append-only ballots, a
    unique `(poll, voter)` constraint, tally computed by **aggregation** over
    ballots (not a mutable counter), regular exports/backups. Then a concurrency
    hiccup can't silently corrupt a result — worst case, recompute.
  - **LIVE queries validated by a throwaway spike (2026-07)** — SurrealDB 2.6.1,
    Rust SDK over WS. Results: (a) correct, ordered Create/Update/Delete
    notifications; (b) concurrent live queries multiplex with no cross-talk;
    (c) events that occur while a client is disconnected are **not** backfilled;
    (d) on a real server drop the stream cleanly **errors then ends** (detectable),
    the SDK **auto-reconnects the handle**, but the old live query does **not**
    resume — you must re-subscribe. Net: the failure mode is clean and detectable,
    not a silent-dead-stream gotcha. The **one required pattern is
    refetch-on-reconnect** (re-`SELECT` current state + issue a fresh `LIVE`
    query) — table stakes for any push sync, not a SurrealDB quirk. Verdict:
    solid enough to build the sync layer on.
  - License: BSL 1.1 — fine self-hosted (converts to Apache 2.0 after 4 years).

- **Conservative alternative — PostgreSQL + Rust extensions.** If the SurrealDB
  bet feels too loose: Postgres stays primary (proven correctness) and you add
  **pgvector** (permissive baseline) → **VectorChord** (Rust, faster) and/or
  **ParadeDB `pg_search`** (Rust, Tantivy BM25 + hybrid) for AI/search, with
  `LISTEN/NOTIFY → WS` for realtime and a hand-rolled sync layer. One DB, proven,
  but you build the sync engine yourself. (VectorChord/ParadeDB are AGPL-3.0 —
  fine self-hosted.)

- **Only if AI becomes a core product surface:** a dedicated vector store —
  **Qdrant** (Rust, Apache-2.0, best-in-class filtered search) or **LanceDB**
  (Rust, Apache-2.0, embedded "SQLite for vectors"). Start with vectors in the
  primary DB; extract later.

- **Only if feeds/timelines become core & freshness-critical:** **RisingWave**
  (Rust, Apache-2.0, incremental materialised views) as a derived-view layer, not
  a replacement.

- **Not worth it here:** Scylla/Cassandra (Bluesky-scale), TiKV/TiDB (ops
  overhead), Neon (self-host complexity). CozoDB (embeddable Rust graph+vector) is
  interesting but young.

Note: "Facebook-like social" is **not** a DB-forcing change at this scale — a
social graph + feeds run fine on any of the above; feed fan-out is an
architecture choice, not a DB pick. And if "AI" means LLM features (chat/summary)
rather than semantic search, that's a compute/API concern — the DB just stores
embeddings + content.

### Other components

- **HTTP: axum** (keep — already used, the Rust standard).
- **atproto SDK: `atrium`** (`atrium-api` / `atrium-oauth` / `atrium-xrpc`) —
  lexicon → Rust codegen, XRPC, and OAuth (replaces the hand-rolled DPoP/PKCE).
- **Firehose: consume Jetstream** (JSON over WS), filtered to your collections —
  simpler than the raw CBOR `subscribeRepos`.
- **Deploy: a persistent process** (small always-on VM/container), **not**
  serverless — the AppView holds a live firehose connection + a WS/live-query
  server + the DB. This is a shift from the current Scaleway serverless-container
  model. Keep Nix + rustls.

## 6. Rough shape

```text
 user repos (public records) ──firehose/Jetstream──▶ ┌──────────────────────────┐
                                                      │  Custom Rust backend      │
 DID identity (atproto OAuth) ───────────────────────▶│  (AppView + org authority)│
                                                      │                          │
                                                      │  new DB (Turso):          │
                                                      │   • materialised records  │
                                                      │   • org-authoritative     │
                                                      │     state (roster, perms, │
                                                      │     tally, ballots, agenda)│
                                                      └───────────┬──────────────┘
                                                        axum WS   │ (realtime sync)
                                                                   ▼
                                                            Dioxus client
```

## 7. Pivotal decisions (deliberately open)

1. **How much lives in atproto records at all?** Given records are public, this is
   probably "identity + public artifacts only," with the private assembly machinery
   in the backend DB. Decide how atproto-native to be.
2. **Run your own PDS, or ride users' existing PDSes?** Own PDS if you need
   private/org-controlled records or members without Bluesky accounts; skip it if
   atproto is just identity + public publishing.
3. **SurrealDB primary, or Postgres primary?** The recommendation above leans
   SurrealDB for the non-critical project (its live queries are the sync engine),
   with Postgres + Rust extensions as the conservative fallback.

## 9. Transition-easing steps (do during the interim)

The cheapest cutover is an incremental one. Maximise what transfers, and
pre-populate the data the new world needs.

1. **Design the domain model + atproto lexicons now (highest leverage).** The
   data model is the one artifact that transfers 100%. Writing the lexicons
   (record schemas for `com.example.wiki.*`) forces the hardest decision — what's a
   *public* record vs *private* authoritative state (§4 split + privacy pivot) —
   while it's still cheap, on paper. Bonus: `atrium` codegens Rust types straight
   from lexicons. A first cut is derivable from the current `mimeId` taxonomy +
   the public/private split.
2. **Sketch the data-migration path (current Postgres → new model).** The current
   data (~4k nodes + members + Slate content) has to move. Writing the mapping now
   — node tree → records + authoritative tables — surfaces model mismatches early
   and yields a runnable migration. Content is already portable Slate JSON.
3. **Pre-populate identity bindings — IF members will hold DIDs.** The member→DID
   pivot is the riskiest migration. If members can get DIDs before cutover
   (especially if you run your own PDS that issues them), start capturing the
   member↔DID binding in the interim app now (atproto OAuth linking already
   exists) so the map fills incrementally instead of a mass re-link at cutover. If
   DIDs are only issued at cutover, at least finalise the email→DID claim-flow
   design now. Depends on pivotal decision §7.2.
4. **Decouple the frontend's data-access seam — IF the Dioxus frontend survives.**
   If you keep the Dioxus UI and only swap the backend, have components read
   *domain types* through a repository/service boundary rather than calling
   `graphql.rs` / cynic types directly; then the backend swap is contained to that
   layer, not every component. If the frontend is also rewritten, skip it.
5. **Freeze discipline.** No new Hasura-coupled features (RLS/computed-field/
   subscription-heavy) that you'll only have to unwind. Every new `mimeId` kind
   should map to a planned lexicon.

## 8. Related notes

- Security findings (fixed vs deferred) and the secret-ballot de-anonymisation
  concern that the rewrite's tallying service must solve: see the security notes.
- Performance: the current stack's costs (Hasura subscription polling, per-row
  computed fields) are Hasura artifacts that do **not** carry over — another
  reason not to optimise the interim.
