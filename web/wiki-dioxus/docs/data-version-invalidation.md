# DATA_VERSION cache invalidation — how it works, and how to scope it

Status: **working as designed; scoping is deferred.** This note is for future
research if the coarse invalidation ever becomes a performance problem. There is
no correctness bug today — only over-fetching.

## How it works

The app has one app-wide "cache epoch": a single reactive counter that every
read-side data query subscribes to, and that every write bumps.

- **The counter** (`src/session.rs`):

  ```rust
  pub static DATA_VERSION: GlobalSignal<u32> = Signal::global(|| 0);

  pub fn bump_data_version() { *DATA_VERSION.write() += 1; }
  ```

- **Every read subscribes to it** via the `use_data_resource!` macro
  (`src/session.rs`), which wraps Dioxus's `use_resource` and folds a *read* of
  `DATA_VERSION()` into the resource's reactive dependency set:

  ```rust
  // explicit-deps form
  let __data_version = DATA_VERSION();                 // read == subscribe
  use_resource(use_reactive!(|(deps.., __data_version)| { /* query */ }))

  // plain-closure form
  use_resource(move || { let _ = DATA_VERSION(); /* query */ })
  ```

- **Why a bump refetches everything:** `use_resource` re-runs its future whenever
  any signal it read last run changes. Because *every* `use_data_resource!` reads
  the *same* global `DATA_VERSION`, incrementing it marks every mounted data
  resource dirty at once, so they all re-query. One `bump_data_version()` refreshes
  the whole view with no reload.

This is the "refetch on write" layer. Separately, a few genuinely live views use
GraphQL **subscriptions** via `use_live` (`src/subscription.rs`) for push updates:
the open poll's tally (`vote.rs`), the speaker list (`speak.rs`), and the
screen/follow views' `active` relation (`screen.rs`).

### What bumps the counter

Roughly 24 sites, all after a successful mutation. Grouped:

- **Session-wide (must stay global):** login (`auth.rs`), logout (`layout.rs`),
  pull-to-refresh (`pull_refresh.rs`).
- **Node-tree / folder-scoped:** editor save (`editor.rs`), sort save (`sort.rs`),
  folder new-child insert + attachable-lock toggle (`folder.rs`), deep-copy paste
  (`folder.rs`), content delete (`content.rs`), file delete (`file.rs`).
- **Vote/poll (context-scoped):** add/delete question, propose amendment, close
  poll, create poll (`vote.rs`). NB a plain vote **cast** does **not** bump
  globally — `PollApp` uses a local `refresh` signal + a `use_live` subscription.
- **Member (context-scoped):** save member edit, invite, bulk roster import,
  single invite, remove member, promote/activate toggles (`member.rs`), and
  accept/decline invitation (`layout.rs`).

### Who refetches on every bump

Every `use_data_resource!` consumer, app-wide, including the highest-fanout ones:
the page resolver (`loader.rs`), the drawer tree (`layout.rs`), the contexts list
(`layout.rs`), the social feed (`social.rs`), comment threads (`comments.rs`),
folder children (`folder.rs`), the member roster (`member.rs`), and every visible
admin poll tally (`admin.rs`). So editing one member's name re-runs all of them.

## The cost

The counter has no notion of *which* node, context, or table changed, so a single
mutation refetches every mounted resource, not just the affected ones. That is the
over-fetching (extra latency + Hasura load + wasm work) that scoping would remove.

## Why it is not already scoped (the load-bearing constraint)

The global bump is doing double duty: it is also the **cross-view consistency**
mechanism for views that have **no** live subscription. The clearest case is
`AdminApp` (the results table, `admin.rs`): it has no `use_live` and no local
refresh, so its poll tallies stay current *only because* an unrelated global bump
reaches them. Narrow the bump and those views go stale until a reload.

Several views also legitimately span **multiple** contexts — the contexts list and
drawer tree (`layout.rs`), the social feed (`social.rs`), profile memberships
(`profile.rs`), recent items (`home.rs`), the page resolver (`loader.rs`) — so
scoping their invalidation to a single context id would *miss* updates.

## Proposed low-regression scoping (if perf demands it)

Additive, opt-in, with the global bump kept as a fallback so nothing regresses
until a call site is deliberately migrated.

1. **Add a per-context registry alongside the global counter:** keep
   `DATA_VERSION` as a global *epoch*; add e.g.
   `CONTEXT_VERSIONS: GlobalSignal<HashMap<String, u32>>` plus
   `bump_context_version(ctx_id)`.
   - Caveat: a `HashMap`-in-a-`GlobalSignal` has whole-map reactivity (any change
     wakes every reader), which defeats the point. Prefer a registry of
     *per-context* `Signal`s (a lazily-populated map of signals), so each context's
     resources subscribe to a distinct signal.
2. **Add a `scope =` arm to `use_data_resource!`** that reads BOTH the global epoch
   AND that scope's counter. Existing (unscoped) call sites are left exactly as-is:
   they keep reading only the global epoch, so they refetch on every global bump
   exactly like today.
3. **Keep `bump_data_version()`** bumping the global epoch, so any un-migrated bump
   site and the must-stay-global sites still invalidate everything.
4. **Migrate incrementally**, one context-local flow at a time, cheapest and
   highest-fanout first (folder insert/edit, member edits). A scoped bump then
   increments only that context's counter, sparing unrelated contexts' resources.

Because each scoped resource reads *both* the global epoch and its scope counter,
correctness degrades gracefully: the worst case is an over-refetch (today's
behavior), never a stale view.

### Do NOT scope these

- `auth.rs` login, `layout.rs` logout — must **flush all scopes** (clear the
  registry + bump the epoch) so no previous-session data lingers.
- `pull_refresh.rs` — a deliberate whole-view refresh.
- Cross-context consumers (contexts list, drawer tree, social feed, profile
  memberships, recent, page resolver) — leave UNSCOPED so they keep refetching on
  every bump.
- Any vote bump (`vote.rs` create/close) — do **not** narrow until `AdminApp` gets
  its own `use_live` subscription on the context's vote nodes (mirror
  `PollApp`'s), or its tallies go stale.

### Verification

Reactivity regressions here are **only** observable in a real browser — the Servo
test harness has missed exactly this class of breakage before (see the
`use_reactive` vs keyed-remount note in `loader.rs`). So any migration must be
verified interactively, not just by the unit tests.

### Rejected alternative

Keying resources by a content hash instead of a version counter — fights
`use_resource`'s identity model and risks *lost* invalidations (a stale view),
which is worse than over-fetching.
