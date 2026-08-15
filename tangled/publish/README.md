# tangled/publish

Publishes subprojects of this monorepo as standalone repos on Tangled, with
real history, using [josh](https://josh-project.github.io/josh/).

The monorepo is the source of truth. Published repos are **read-only mirrors**:
each run filters monorepo history and force-pushes the result, so anything
committed directly to a published repo is destroyed on the next run.

## Usage

```sh
nix shell nixpkgs#josh                     # provides josh-filter
nu publish.nu --dry-run                    # filter everything, push nothing
nu publish.nu --only rust-awk              # one project
nu publish.nu --ssh-key ~/.ssh/id_ed25519  # filter and push what changed
```

Projects and their filters live in [`projects.nuon`](./projects.nuon).
Nineteen are published today; adding one is three steps:

```sh
tangled-cli repo create rust-foo --knot knot1.tangled.sh --description "..."
tangled-cli repo add-collaborator overby.me/rust-foo agent.overby.me   # so CI can push
# add the entry to projects.nuon, then:
nu publish.nu
```

The collaborator step is not optional and is easy to forget: without it the
push fails with "you aren't authorized to push to this repository". A repo's
collaborators live on the knot rather than as records on the owner's PDS, so
before `repo add-collaborator` existed this needed the web UI once per repo,
which is what made scaling out awkward.

Filtering all nineteen takes about 13 seconds cold, and re-running with
nothing to do costs two `ls-remote` calls per project. No knot rate limiting
has been observed at this size.

## Taking a contribution back

Published repos are read-only mirrors, so a contribution arrives as a branch
on one of them (typically a pull request's source branch). `ingest`
reverse-filters that branch back through the project's filter and lands it on
a local review branch, re-prefixed into the project's directory:

```sh
nu publish.nu ingest rust-awk --from-ref some-contribution
```

```text
ingested 1 commit(s) from rust-awk some-contribution
  downstream tip: e64e13d767
  review branch:  ingest/rust-awk at 6e07883c84

6e07883c8 doc: add a contributing guide
 rust/awk/CONTRIBUTING.md | 3 +++
```

Nothing is pushed and nothing lands on `main`: the review branch is built in
the mirror for you to fetch, read, and merge yourself. It is parented on the
current monorepo branch, so it merges as a fast-forward, and josh rebases it
if the monorepo moved on in the meantime.

This is why the mirrors can stay read-only and force-pushed. The alternative,
letting the sync job write to the monorepo, would need the CI bot to hold
write access to `main` — and Tangled has no branch protection to contain the
damage if that went wrong.

## Why josh-filter and not josh-proxy

A proxy serves filtered *views* at URLs. Those are not repos on the forge: no
issues, no pulls, no discoverability. We want real Tangled repos, so we filter
locally and push.

A proxy would answer a different question anyway: working *in* one project as
a slice day to day, rather than distributing projects. rust-lang's josh-sync
does exactly that, and notably runs josh-proxy as a local subprocess rather
than hosting it, so wanting a slice would not mean running a service.

**It does not currently work against Tangled**, for two independent reasons,
both measured:

- josh addresses a filtered view as `<upstream-path>.git:<filter>.git`, so it
  splits the URL on `:`. A knot addresses repos by DID over HTTP
  (`did:plc:...`), which is full of colons, and josh parses the DID as part of
  the path: asked for `did:plc:….git:/rust/awk.git` it fetched the knot's
  `/rust/awk.git`, which is nothing. Percent-encoding does not help.
- The appview (`tangled.org/<owner>/<repo>`) has colon-free URLs and clones
  fine, but does not carry git's auth challenge through to the knot, so a push
  through it fails with `missing authorization header`. Clone-only is useless
  for a workflow whose whole point is pushing back.

So a slice workflow needs either josh to accept an escaped path, or the
appview to pass auth through. Until one of those, filter-and-push is not just
the simpler option, it is the only one.

## Design notes

**One clone, reused.** josh's filter cache is per-repo local state and
successive filters share it. Cloning per project throws that away and hammers
the knot. The mirror lives under `--work-dir` (default
`$XDG_CACHE_HOME/tangled-publish`) and is fetched, not recloned, on later runs.

**Never in a working repo.** `josh-filter` has no flag to choose a repository:
it operates on the current working directory, and writes both refs and a cache
into whatever repo that is. Run it against a jj checkout and you get
`refs/josh/*` and `.git/josh/` in it. (`--repo` is for the GraphQL/query API,
not for this.)

**Controlled git config.** The script passes `-c fetch.prunetags=false -c
gc.auto=0` on every git invocation rather than inheriting whatever the caller
has. `fetch.prunetags` in particular can make josh operations silently do
nothing.

**Change detection asks the remote.** The filtered SHA is compared against the
remote's current branch head via `ls-remote`, not against local state, so a
cold CI runner with an empty work dir still skips unchanged projects.

**Force-push is correct here.** This tree is developed with jj; amending a
synced commit rewrites every filtered commit below it. That is fine for a
read-only mirror, and is the other reason mirrors must stay read-only.

## Filter conventions

```text
:/<path>            subdirectory becomes the repo root
:exclude[::*.nix]   drop every top-level nix file: default.nix is a
                    flakelight module the root flake imports, reaching for
                    ../../nix/lib/cargo/index, and the testsuite helpers it
                    calls are dead weight without it. A glob rather than a
                    list, because several projects carry more than two
                    (rust/pipewire has six) and new ones appear.
:unsign             strip signatures; these are publish-only mirrors
```

Switching the pilots from two explicit excludes to the glob produced byte
identical output, so it rewrote no published history.

**Keep the shared surface empty.** Every path mapped into a published repo
makes it pick up commits touching that path. Map `flake.lock` into N projects
and one `nix flake update` becomes N commits across N repos, forever. No
current filter maps anything from outside its own directory.

## Known wart: empty commits

The monorepo contains 28 commits with no file changes (jj artifacts). josh
reproduces them faithfully, so each published repo carries the ones on its
branch, with subjects about unrelated projects. `rust-awk`'s tip is one. The
fix would be rewriting monorepo history, which is not worth it.

## Version pin

Filters are verified against `josh-filter r26.05.08`, the version in the
nixpkgs pinned by this repo's `flake.lock`. josh's filter language has had
breaking changes (`:rev` reworked, `:from` removed, cache path moved) and
older versions do not always round-trip losslessly. The script warns on a
version mismatch.

Round-trip was verified with:

```sh
josh-filter ':/rust/awk' HEAD --update refs/josh/awk
josh-filter ':/rust/awk' HEAD --update refs/josh/awk --reverse --check-roundtrip
```

Note the forward run must come first: `--reverse` reads the ref it is told to
update, and panics if it does not exist yet.

## Creating the repo

Use `tangled-cli repo create <name> --knot knot1.tangled.sh`. Two things about
that flow are load-bearing, and getting either wrong produces a repo that
pushes fine over SSH and 404s on the web:

- **The record key is the repo's identity.** `sh.tangled.repo` declares
  `key: "any"` and describes `name` as only a "Cosmetic name of the repo", so
  the appview addresses a repo as `<handle>/<rkey>`. The rkey must be the
  name; a PDS-assigned TID is invisible.
- **The knot mints the repo's own DID** and returns it from
  `sh.tangled.repo.create`. The record has to carry it as `repoDid`, so the
  knot is called before the record is written.

Re-running `repo create` for an existing name mints a *new* repo DID and a
fresh empty repo on the knot, orphaning the old one. Re-run `publish.nu`
afterwards to refill it.

## Authentication

A knot authorises a push in one of two ways. There are no deploy keys and no
branch protection in either case: authorisation is per repo, not per ref, and
a force-push is always accepted.

**SSH**, which this script uses: the offered key must be published by an
account allowed to write to that repo (a `sh.tangled.publicKey` record on that
account's PDS). `--ssh-key` is passed with `IdentitiesOnly=yes`, because
offered several keys a knot rejects on the first non-matching one and gives
up. CI therefore needs a private key in a Spindle secret, belonging to such an
account.

**HTTP**, which an automated client or a josh-proxy would use: pass an atproto
service-auth JWT, either as `Authorization: Bearer <jwt>` or as the *password*
of a basic credential whose user is `x-tangled-token`. Mint it with
`com.atproto.server.getServiceAuth` using `aud=did:web:<knot>` and
`lxm=sh.tangled.repo.push`. The knot caps the token's lifetime at **300
seconds**, though it may be reused until it expires. Repos are addressed by
their own DID over HTTP: `https://<knot>/<repoDid>`.

(Older Go knotservers refuse HTTP push outright with "Pushes are only
supported over SSH". `knot1.tangled.sh` runs the newer Rust knot, which does
not.)

300 seconds is too short to paste by hand, so `tangled-cli` ships a git
credential helper that mints one per push. The empty value first is
load-bearing: it resets the inherited helper list for that URL, without which
a global helper such as `store` writes the token to `~/.git-credentials` in
plaintext and then replays it after it has expired.

```sh
git config --global credential.https://knot1.tangled.sh.helper ''
git config --global --add credential.https://knot1.tangled.sh.helper \
    '!tangled-cli git-credential'
```
