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

## Why josh-filter and not josh-proxy

A proxy serves filtered *views* at URLs. Those are not repos on the forge: no
issues, no pulls, no discoverability. We want real Tangled repos, so we filter
locally and push.

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
:/<path>                   subdirectory becomes the repo root
:exclude[::default.nix]    drop the flakelight module (meaningless outside
                           the monorepo: the root flake imports it and it
                           reaches for ../../nix/lib/cargo/index)
:exclude[::testsuite.nix]  drop the nix check helper default.nix called
:unsign                    strip signatures; these are publish-only mirrors
```

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

Tangled push is **SSH only**. There are no deploy keys, no per-repo tokens and
no HTTPS push: a knot authorises a push by matching the offered SSH key
against keys registered to accounts that may write to that repo. So CI needs a
private key in a Spindle secret, belonging to an account with push rights.

`--ssh-key` is passed with `IdentitiesOnly=yes`, because offered several keys
a knot rejects on the first non-matching one and gives up.
