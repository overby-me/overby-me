# tangled-cli feature plan

Feature additions inspired by the GitHub CLI (`gh`), prioritized by workflow impact.

## High priority — daily workflow

### 1. `browse` — open in browser

Open a repo, issue, or PR in the web browser.

```text
tangled browse                          # current repo (detect from git remote)
tangled browse overby.me/core           # repo page
tangled browse overby.me/core --issues  # issues tab
tangled browse overby.me/core --prs    # PRs tab
tangled browse --issue 3mh4gldqpez2l    # specific issue
tangled browse --pr 3mgz7n647kv22       # specific PR
```

Implementation:

- Construct `https://tngl.sh/{handle}/{repo}` URLs (and `/issues/{rkey}`, `/pulls/{rkey}` variants)
- Use the `open` crate (already a dependency) to launch the browser
- Detect repo from git remote origin when no argument given
- Add `--print` / `-n` flag to print URL instead of opening

### 2. `repo edit` — modify repo settings

```text
tangled repo edit overby.me/core --description "New description"
tangled repo edit overby.me/core --private
tangled repo edit overby.me/core --public
```

Implementation:

- Fetch current `sh.tangled.repo` record via `com.atproto.repo.getRecord`
- Modify fields and write back via `com.atproto.repo.putRecord`
- Support: `--description`, `--private`/`--public`, `--default-branch`

### 3. `issue close` / `issue reopen`

Explicit subcommands instead of `issue edit --state closed`.

```text
tangled issue close 3mh4gldqpez2l
tangled issue close 3mh4gldqpez2l --comment "Fixed in abc123"
tangled issue reopen 3mh4gldqpez2l
```

Implementation:

- Thin wrappers around existing `close_issue` / state-change logic
- `--comment` flag posts a comment atomically with the state change

### 4. `issue delete`

```text
tangled issue delete 3mh4gldqpez2l
tangled issue delete 3mh4gldqpez2l --force  # skip confirmation
```

Implementation:

- Delete via `com.atproto.repo.deleteRecord` on the issue record
- Prompt for confirmation unless `--force`

### 5. `pr close` / `pr reopen`

Close a PR without merging, or reopen a closed PR.

```text
tangled pr close 3mgz7n647kv22
tangled pr close 3mgz7n647kv22 --comment "Superseded by ..."
tangled pr reopen 3mgz7n647kv22
```

Implementation:

- Update PR state record (similar to issue state change pattern)
- `--comment` flag posts a review comment with the close

### 6. `pr comment`

Standalone comment on a PR (distinct from formal review).

```text
tangled pr comment 3mgz7n647kv22 --body "Looks good, just one nit"
```

Implementation:

- Reuse existing `comment_pull` API method
- More intuitive than `pr review --comment "..."` for non-review discussion

### 7. `pr diff`

Standalone diff viewer (currently only `pr show --diff`).

```text
tangled pr diff 3mgz7n647kv22
tangled pr diff 3mgz7n647kv22 --color
```

Implementation:

- Fetch PR record patch and print to stdout
- Pipe-friendly (no extra formatting unless `--color`)

## Medium priority — power user features

### 8. `api` — raw authenticated XRPC calls

Make arbitrary authenticated API requests. The swiss army knife.

```text
tangled api get sh.tangled.repo.languages --param did=did:plc:... --param name=core
tangled api get com.atproto.repo.listRecords --param repo=did:plc:... --param collection=sh.tangled.repo
tangled api post sh.tangled.repo.create --input body.json
```

Implementation:

- `get` / `post` subcommands
- `--param key=value` for query params (GET) or JSON fields (POST)
- `--input <file>` for POST body (or stdin with `-`)
- Output raw JSON to stdout (pipeable to `jq`)
- Uses current session auth automatically
- Targets the Tangled API base URL by default, `--pds` flag for PDS calls

### 9. `status` — cross-repo dashboard

Morning coffee command: what needs my attention?

```text
tangled status
```

Output:

```text
Issues assigned to you:
  overby.me/core#3mh4g  Bug in auth flow

PRs awaiting your review:
  platform/tangled/core#3mgz7n   Add AVIF support

Your open PRs:
  platform/tangled/core#3mgfge   Add shebang detection
```

Implementation:

- List user's issues (assigned), PRs (authored, open), PRs (review requested)
- Aggregate across repos
- Color-coded by state

### 10. `pr checkout`

Apply a PR's patch locally for testing.

```text
tangled pr checkout 3mgz7n647kv22
tangled pr checkout 3mgz7n647kv22 --branch pr-review
```

Implementation:

- Fetch PR patch from record
- Create a local branch and apply via `git am`
- Default branch name: `pr/{rkey}`

### 11. `repo fork`

Fork a repository (if Tangled supports this).

```text
tangled repo fork overby.me/core
tangled repo fork overby.me/core --name my-fork
```

Implementation:

- Create new repo on user's account
- Seed from source repo URL
- Depends on Tangled API support for fork metadata

## Known limitations

### Appview indexing for CLI-created repos

Repos created via CLI (`repo create`, `repo fork`) are correctly stored on the PDS and the knot server, but won't immediately appear on `tangled.org`. The tangled appview maintains its own SQLite database and only populates it through its web form handlers — it does not subscribe to `sh.tangled.repo` records via the Jetstream firehose. Git operations (clone, push, pull) work fine for CLI-created repos; only the web UI listing is affected.

Upstream fix needed: the tangled appview should either add `sh.tangled.repo` to its Jetstream subscription or expose an XRPC endpoint for repo registration.

## Lower priority — nice to have

### 12. `search`

Search across Tangled.

```text
tangled search repos nix
tangled search issues "build failure" --repo overby.me/core
tangled search prs --author overby.me --state open
```

Implementation:

- Depends on Tangled having search API endpoints

### 13. `label`

Manage labels on issues/PRs (if Tangled supports labels).

```text
tangled label list --repo overby.me/core
tangled label create --repo overby.me/core --name bug --color red
```

### 14. `repo rename`

```text
tangled repo rename overby.me/old-name new-name
```

Implementation:

- Update PDS record name field
- May need knot-side rename support

### 15. Shell completions

```text
tangled completion bash > /etc/bash_completion.d/tangled
tangled completion zsh > ~/.zfunc/_tangled
tangled completion fish > ~/.config/fish/completions/tangled.fish
tangled completion nushell > ~/.config/nushell/completions/tangled.nu
```

Implementation:

- clap has built-in `clap_complete` support
- Add `completion` subcommand that generates shell-specific scripts
