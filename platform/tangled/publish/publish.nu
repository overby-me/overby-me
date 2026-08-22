#!/usr/bin/env nu

# Publish monorepo subprojects as standalone Tangled repos.
#
# One-way: the monorepo is the source of truth and every published repo is a
# read-only mirror. Each run filters the monorepo history with josh-filter and
# force-pushes the result. Force is expected: this tree is developed with jj,
# so amending a synced commit rewrites every filtered commit below it. Nobody
# may commit directly to a published repo.
#
# Everything happens in a dedicated plain-git clone under --work-dir, never in
# a working repo. josh-filter has no flag to choose a repository: it operates
# on the current working directory, and it writes both refs and a cache into
# whatever repo that is. Pointing it at a jj repo pollutes `jj op log`.
#
#   nu publish.nu --dry-run              # filter everything, push nothing
#   nu publish.nu --only oxidized-awk        # one project
#   nu publish.nu                        # filter and push what changed

# josh's filter language has had breaking changes (`:rev` reworked, `:from`
# removed, cache path moved), and older versions do not always round-trip
# losslessly. Pin to what the filters were designed and verified against.
# This is the version in the nixpkgs pinned by the repo's flake.lock, so a
# CI runner and a laptop get the identical binary.
const JOSH_VERSION = "r26.05.08"

# git config that must not be inherited from whoever runs this.
# fetch.prunetags in particular can make josh operations silently do nothing.
const GIT_CONFIG = [
  "-c" "fetch.prunetags=false"
  "-c" "gc.auto=0"
  "-c" "core.hooksPath=/dev/null"
  "-c" "advice.detachedHead=false"
]

# Run git in $repo with the controlled config, returning the completion record.
# Takes its arguments as a list: a rest parameter would swallow `--force` and
# friends as flags of this command.
def git-in [repo: path, args: list<string>]: nothing -> record {
  ^git -C $repo ...$GIT_CONFIG ...$args | complete
}

# Strip credentials out of anything that might be shown or logged. The GitHub
# remote carries a token in its URL, and both the failing command and git's own
# stderr would otherwise reproduce it in CI output.
def redact []: string -> string {
  $in | str replace -r -a '://[^@/]+@' '://***@'
}

# Run git and fail loudly, returning trimmed stdout.
def git-ok [repo: path, args: list<string>]: nothing -> string {
  let out = (git-in $repo $args)
  if $out.exit_code != 0 {
    let what = ($args | str join ' ' | redact)
    error make {msg: $"git ($what) failed: ($out.stderr | str trim | redact)"}
  }
  $out.stdout | str trim
}

# The GitHub mirror pushes over HTTPS with a token rather than over SSH,
# because --ssh-key forces IdentitiesOnly for the knot: that same key would
# then be the only one offered to GitHub too.
def github-token []: nothing -> string {
  let from_env = ($env.GITHUB_TOKEN? | default "")
  if not ($from_env | is-empty) { return $from_env }
  let gh = (^gh auth token | complete)
  if $gh.exit_code != 0 {
    error make {msg: "no GitHub token: set $GITHUB_TOKEN or run `gh auth login` (or pass --no-github)"}
  }
  $gh.stdout | str trim
}

# The filter a project publishes under, from what it is and where it has been.
#
# Not stored: a filter is derivable, and the stored ones drew blood twice -
# one silently dropped a dependency, another silently mapped in an unrelated
# project. What cannot be derived is the history, because josh reconstructs
# it: a filter naming only today's path publishes a repo whose history starts
# at the last move. So `formerPaths` records where a project has been, oldest
# first, each with the siblings it depended on at the time (a dependency
# moves house too, so the pairing is per era rather than global).
#
# `kind` is what the project is made of. A crate's .nix files are the
# monorepo's build of it, which the published repo replaces with a generated
# flake, so they are excluded. A nix project's .nix files ARE the project, so
# only its workflow is replaced.
#
# The generated files - the flake, its lock, the CI workflow - are not kept
# in the project's directory. They live together under generated/ and josh
# maps them into position, because `::destination=source` places a file
# somewhere other than where it was found. Three things follow from that.
# They stop being 115 inert files scattered through the tree that look
# editable and are not; the workflow stops being 39 byte-identical copies and
# becomes one file mapped 39 times; and a generated file can no longer drift
# from the filter that ships it, because the same expression names both.
#
# `--current` drops the former eras and keeps only today's paths. Publishing
# wants every era, because that is what reconstructs the history. Reversing
# wants none of them: josh attributes a reversed tree to the *first* matching
# subtree term, which is the oldest era, so ingesting a contribution to
# oxidized-sed through the full filter writes it to rust/sed - a directory
# that moved house two renames ago. A contribution is against today's tree, so
# it should be filtered by today's paths. The two filters produce the same
# tree at the tip, because an era's directory no longer exists there; that is
# checked below rather than assumed.
def derive-filter [p: record, --current]: nothing -> string {
  let kind = ($p | get -o kind | default "crate")
  let gen = "platform/tangled/publish/generated"
  let today = {path: $p.path, deps: ($p | get -o deps | default [])}
  let eras = if $current { [$today] } else {
    ($p | get -o formerPaths | default []) | append $today
  }
  # Excluded from every era, not just today's: an era predating the move
  # still has these files in its directory, and letting them through would
  # publish two of each, the stale one winning wherever it is prefixed.
  let generated = ":exclude[::flake.lock]:exclude[::.tangled/]"
  # Files of the project's directory that are about the monorepo rather than
  # about the project, and so do not belong in a clone of it. A nix project
  # ships its directory as it stands, which is the only kind that can hold
  # one: a crate's .nix files are dropped wholesale already.
  let dropped = ($p | get -o exclude | default [] | each {|e| $":exclude[::($e)]"} | str join "")
  # The monorepo's build of a crate: every nix file of the project's own
  # directory, and the default.nix of any directory below it. `*.nix` matches
  # one path segment, so alone it leaves the nested ones behind - and a
  # nested default.nix is a project nix-workspace discovers once the repo
  # root IS this directory, importing a module written to be found by the
  # monorepo. Only that name recurses: a .nix file deeper in the tree that is
  # not a default.nix is the project's own (systemd-rs carries 510 NixOS VM
  # tests), and dropping those would publish a repo missing its testsuite.
  let nix_dropped = ":exclude[::*.nix]:exclude[::**/default.nix]"
  let terms = ($eras | each {|era|
    let path = $era.path
    let deps = ($era | get -o deps | default [])
    let base = ($path | path basename)
    # An era can be told to drop a directory as well, for the one move
    # `formerPaths` does not otherwise survive: a project that moved *into* a
    # subdirectory of where it was. Every other former path stopped existing
    # at the move, so its term yields nothing at the tip and history is all it
    # contributes. This one still exists and now holds the new location, so
    # without dropping it the era republishes today's tree one level down -
    # and it wins, because a union takes both and this one has the whole
    # project inside a directory named after where it went.
    let era_dropped = ($era | get -o exclude | default [] | each {|e| $":exclude[::($e)]"} | str join "")
    if $kind == "nix" {
      [$":/($path):exclude[::.tangled/]($dropped)($era_dropped)"]
    } else if ($deps | is-empty) {
      # `nix_dropped` drops the monorepo's build of the project, then
      # flake.nix is let back through: it is the published repo's own build,
      # and since the framework left this tree it is the same file here and
      # there rather than one mapped over the other.
      # Module files named in `nixosModules` are the project's own
      # interface, so they ship the way flake.nix does.
      ([$":/($path)($nix_dropped)($generated)" $":/($path)::flake.nix"]
        | append (($p | get -o nixosModules | default {} | values)
          | append ($p | get -o homeModules | default {} | values)
          | each {|f| $":/($path)::($f)"}))
    } else {
      # Each directory keeps its own name at the root, so a Cargo.toml's
      # `path = "../pcre2"` still resolves once they are side by side. The
      # project's own flake and README are lifted back to the repo root.
      [$":/($path)($nix_dropped)($generated):exclude[::README.md]:prefix=($base)"]
      | append ($deps | each {|d| $":/($d)($nix_dropped):prefix=($d | path basename)" })
      | append [$":/($path)::flake.nix" $":/($path)::README.md"]
    }
  } | flatten)
  # One workflow for every project; a flake and a lock for those that have a
  # generated one. A nix project's flake is its own source, not an artifact.
  #
  # Each is mapped from every home it has had: the project's own directory
  # once per era, and generated/ LAST. Naming them all is what keeps the
  # history whole - mapping only from generated/ published a repo whose every
  # historical commit had no flake at all, the same failure `formerPaths`
  # exists to prevent, a repo you can clone but cannot build at any commit
  # before today.
  #
  # The order is load-bearing and was measured, not assumed: where two terms
  # write the same destination, the LAST one wins. For most of history only
  # one source exists and order cannot matter, but a project whose own
  # directory holds a real flake - one that path-inputs nix-workspace so the
  # monorepo can evaluate it - has both at once, and the published repo must
  # get the generated one. That path does not exist in a clone.
  # flake.nix is not among these any more. It names nix-workspace by the URL a
  # clone uses, which is now the only spelling there is: the framework left
  # this tree, so the in-tree file and the published one are the same file and
  # there is nothing to map over. It ships from the project's own directory,
  # let back through the `exclude[::*.nix]` that drops the monorepo's build.
  let artifacts = (
    [{dest: ".tangled/workflows/ci.yml", now: $"($gen)/ci.yml", then: ".tangled/workflows/ci.yml"}]
    | append (if $kind == "nix" { [] } else {
      [{dest: "flake.lock", now: $"($gen)/($p.name)/flake.lock", then: "flake.lock"}]
    })
  )
  let mapped = ($artifacts | each {|a|
    # A project that ships its own workflow overrides the generated one:
    # its current-path term moves after the generated term, where josh lets
    # it win. Moved rather than appended, because josh normalises the
    # filter and a duplicated term collapses back to its first position.
    # Anchored to the repo root, because filters are also derived from
    # other working directories. The workflow ONLY: a project's in-tree
    # flake.lock is the monorepo's pin of it, and letting it win published
    # oxidized-sed against a framework rev its new flake could not call.
    let shipped = (
      $a.dest == ".tangled/workflows/ci.yml"
      and ($env.FILE_PWD | path join "../../.." $p.path $a.then | path expand | path exists)
    )
    let former = ($eras | where {|era| (not $shipped) or $era.path != $p.path })
    ($former | each {|era| $"::($a.dest)=($era.path)/($a.then)" })
    | append [$"::($a.dest)=($a.now)"]
    | append (if $shipped { [$"::($a.dest)=($p.path)/($a.then)"] } else { [] })
  } | flatten)
  $":[(($terms | append $mapped) | str join ',')]:unsign"
}

def check-josh []: nothing -> nothing {
  let found = (^josh-filter --version | complete)
  if $found.exit_code != 0 {
    error make {msg: "josh-filter not found on PATH (nix shell nixpkgs#josh)"}
  }
  let version = ($found.stdout | str trim | parse "Version: {v}" | get v.0? | default "unknown")
  if $version != $JOSH_VERSION {
    print $"warning: josh-filter is ($version), filters were verified against ($JOSH_VERSION)"
  }
}

# Create the mirror if absent, then fetch the published branch into it.
#
# Deliberately not `clone --branch <branch>`: a CI checkout is a detached HEAD
# with the branch present only as refs/remotes/origin/<branch>, and cloning
# such a source fails with "Remote branch main not found in upstream origin".
# Fetching an explicit refspec, with the remote-tracking ref as a fallback,
# works for a normal checkout and a CI one alike.
def sync-mirror [work_dir: path, source: string, branch: string]: nothing -> path {
  let mirror = ($work_dir | path join "monorepo.git")
  if not ($mirror | path exists) {
    mkdir $work_dir
    print $"creating ($mirror)"
    let out = (^git ...$GIT_CONFIG init --bare --quiet $mirror | complete)
    if $out.exit_code != 0 {
      error make {msg: $"init failed: ($out.stderr | str trim)"}
    }
  }
  # Re-point origin every run: the source differs between a laptop and CI.
  git-in $mirror ["remote" "remove" "origin"] | ignore
  git-ok $mirror ["remote" "add" "origin" $source] | ignore

  print $"fetching ($branch) from ($source)"
  let local = (git-in $mirror ["fetch" "--force" "origin" $"+refs/heads/($branch):refs/heads/($branch)"])
  if $local.exit_code != 0 {
    let tracked = (git-in $mirror ["fetch" "--force" "origin" $"+refs/remotes/origin/($branch):refs/heads/($branch)"])
    if $tracked.exit_code != 0 {
      error make {msg: $"cannot fetch ($branch) from ($source): ($local.stderr | str trim)"}
    }
  }
  $mirror
}

# Apply one filter. josh-filter prints the resulting commit SHA on stdout and
# needs the repo as its working directory.
def filter-project [mirror: path, name: string, filter: string, rev: string]: nothing -> string {
  let out = (do {
    cd $mirror
    ^josh-filter $filter $rev --update $"refs/josh/($name)" | complete
  })
  if $out.exit_code != 0 {
    error make {msg: $"josh-filter ($filter) failed: ($out.stderr | str trim)"}
  }
  let sha = ($out.stdout | str trim | lines | last)
  if ($sha | str length) != 40 {
    error make {msg: $"josh-filter ($filter) printed no sha: ($out.stdout | str trim)"}
  }
  $sha
}

# What the remote currently has, or null if it has nothing yet. Asking the
# remote rather than trusting local state keeps change detection correct on a
# cold CI runner whose work dir was thrown away.
#
# An unreachable remote is reported, not fatal: it is also what a not-yet-
# created repo looks like. The push that follows fails loudly enough.
def remote-head [mirror: path, remote: string, branch: string]: nothing -> any {
  let out = (git-in $mirror ["ls-remote" $remote $"refs/heads/($branch)"])
  if $out.exit_code != 0 {
    print $"warning: cannot read ($remote): ($out.stderr | str trim | lines | first)"
    return null
  }
  let line = ($out.stdout | str trim)
  if ($line | is-empty) { null } else { $line | split row "\t" | first }
}

# The mirror clone is reused between runs on purpose: josh's filter cache is
# per-repo local state and successive filters share it, so one clone for all
# projects is far cheaper than one clone each, and it does not hammer the knot.
#
# --ssh-user exists because this pushes over SSH, where a knot matches the
# offered key against keys published by accounts allowed to write. In CI that
# is the bot account's handle, not the owner's. The key is forced with
# IdentitiesOnly: offered several keys, a knot rejects on the first
# non-matching one and gives up.
def main [
  --source: string            # monorepo to filter (default: this checkout's origin)
  --work-dir: path            # where the mirror clone and josh cache live
  --owner: string = "overby.me" # Tangled account owning the published repos
  --ssh-user: string          # SSH login to push as (default: --owner)
  --ssh-key: path             # private key to push with
  --branch: string = "main"   # branch to read from, and to publish as
  --only: string              # only this project
  --github: string = "overby-me" # GitHub account to mirror to as well
  --no-github                 # publish to Tangled only
  --dry-run                   # filter and report, push nothing
]: nothing -> nothing {
  check-josh

  let here = ($env.FILE_PWD | default ".")
  let config = ($here | path join "projects.nuon")
  if not ($config | path exists) {
    error make {msg: $"no project config at ($config)"}
  }
  let projects = (
    open $config
    | where {|p| $only == null or $p.name == $only }
  )
  if ($projects | is-empty) {
    error make {msg: $"no projects selected; --only ($only) matched nothing"}
  }

  let source = if $source != null { $source } else {
    ^git remote get-url origin | str trim
  }
  let work_dir = if $work_dir != null { $work_dir } else {
    ($env.XDG_CACHE_HOME? | default ($env.HOME | path join ".cache")) | path join "tangled-publish"
  }

  if $ssh_key != null {
    let user = ($ssh_user | default $owner)
    # ControlPath=none because a multiplexed connection is already
    # authenticated: with ControlMaster on, ssh reuses the existing socket and
    # never consults -i or IdentitiesOnly, so the knot sees whichever key
    # opened it first and answers "this ssh key doesn't match any key
    # published by the accounts that may push here" no matter what is passed.
    $env.GIT_SSH_COMMAND = $"ssh -i ($ssh_key) -o IdentitiesOnly=yes -o ControlMaster=no -o ControlPath=none -o BatchMode=yes -o StrictHostKeyChecking=accept-new"
    print $"pushing as ($user)@tangled.org with ($ssh_key)"
  }

  let mirror = (sync-mirror $work_dir $source $branch)
  let input = (git-ok $mirror ["rev-parse" $branch])
  print $"monorepo ($branch) at ($input)"
  print ""

  # Fetched once rather than per project, and never printed.
  let gh_token = if $no_github or $dry_run { "" } else { github-token }
  if not $no_github { print $"mirroring to github.com/($github)" }

  let ssh_login = ($ssh_user | default $owner)
  let results = (
    $projects | each {|p|
      let remote = $"($ssh_login)@tangled.org:($owner)/($p.name)"
      # Each mirror is pushed the same filtered commit, so the two forges
      # cannot drift apart, and each is reported on its own: a GitHub outage
      # should be visible rather than hide a good Tangled publish.
      let gh_remote = $"https://x-access-token:($gh_token)@github.com/($github)/($p.name)"
      try {
        let sha = (filter-project $mirror $p.name (derive-filter $p) $input)
        let published = (remote-head $mirror $remote $branch)
        let commits = (git-ok $mirror ["rev-list" "--count" $"refs/josh/($p.name)"])
        let short = ($sha | str substring 0..9)

        let tangled = if $published == $sha { "skipped" } else if $dry_run { "would push" } else {
          # Force: rewriting monorepo history rewrites every filtered commit.
          git-ok $mirror ["push" "--force" $remote $"($sha):refs/heads/($branch)"] | ignore
          "pushed"
        }

        let gh = if $no_github { "-" } else if $dry_run { "would push" } else {
          try {
            if (remote-head $mirror $gh_remote $branch) == $sha { "skipped" } else {
              git-ok $mirror ["push" "--force" $gh_remote $"($sha):refs/heads/($branch)"] | ignore
              "pushed"
            }
          } catch {|e| $"FAILED: ($e.msg)" }
        }

        {project: $p.name, output: $short, commits: $commits, tangled: $tangled, github: $gh}
      } catch {|e|
        {project: $p.name, output: "-", commits: "-", tangled: $"FAILED: ($e.msg)", github: "-"}
      }
    }
  )

  print ""
  print $"input ($input | str substring 0..9) -> ($projects | length) projects"
  $results | table | print

  let failed = (
    $results | where {|r|
      ($r.tangled | str starts-with "FAILED") or ($r.github | str starts-with "FAILED")
    }
  )
  if not ($failed | is-empty) {
    # A publish pipeline that fails silently is the failure mode here.
    error make {msg: $"($failed | length) projects failed to publish"}
  }
}

# Print the filter each project publishes under.
#
# A filter is derived rather than stored, which makes it correct by
# construction but invisible: the only way to see one was to read
# derive-filter and run it in your head. Changing that derivation changes
# every published repo, so being able to diff the filters before and after is
# what makes such a change reviewable.
# Printed as plain `name<tab>filter` lines rather than returned as a table:
# a filter is far wider than a terminal, and a rendered table truncates it,
# which would make a diff of the output silently agree.
def "main filters" [
  --only: string              # only this project
  --current                   # today's paths only, as `main ingest` reverses through
]: nothing -> nothing {
  let here = ($env.FILE_PWD | default ".")
  open ($here | path join "projects.nuon")
  | where {|p| $only == null or $p.name == $only }
  | each {|p|
    let f = if $current { derive-filter $p --current } else { derive-filter $p }
    print $"($p.name)\t($f)"
  }
  | ignore
}

# Create the GitHub mirrors that do not exist yet. Kept out of `main` on
# purpose: publishing should push to repos, not bring them into being, and a
# typo in a project name would otherwise quietly create a repo rather than
# fail. Run it once when adding a project.
def "main setup-github" [
  --github: string = "overby-me" # GitHub account to create the mirrors under
  --only: string              # only this project
  --private                   # create them private instead of public
  --dry-run                   # report what is missing, create nothing
]: nothing -> nothing {
  let here = ($env.FILE_PWD | default ".")
  let projects = (
    open ($here | path join "projects.nuon")
    | where {|p| $only == null or $p.name == $only }
  )

  let results = (
    $projects | each {|p|
      let slug = $"($github)/($p.name)"
      let exists = (^gh repo view $slug --json name | complete | get exit_code) == 0
      if $exists {
        {repo: $slug, action: "exists"}
      } else if $dry_run {
        {repo: $slug, action: "would create"}
      } else {
        # The description points home, so the repo says what it is from the
        # search results, before anyone opens the README.
        let desc = $"Read-only mirror of overby.me/overby.me ($p.path), published with josh"
        let vis = if $private { "--private" } else { "--public" }
        let made = (^gh repo create $slug $vis --description $desc | complete)
        if $made.exit_code == 0 {
          {repo: $slug, action: "created"}
        } else {
          {repo: $slug, action: $"FAILED: ($made.stderr | str trim)"}
        }
      }
    }
  )
  $results | table | print

  let failed = ($results | where {|r| $r.action | str starts-with "FAILED" })
  if not ($failed | is-empty) {
    error make {msg: $"($failed | length) repos could not be created"}
  }
}

# Reverse-filter downstream work back into the monorepo.
#
# Published repos are read-only mirrors, so a contribution arrives as a branch
# on one of them (typically a pull request's source branch). This maps that
# branch's commits back through the project's filter and lands them on a local
# review branch in the mirror, re-prefixed into the project's directory.
# Nothing is pushed anywhere: you fetch the branch, read it, and merge it
# yourself.
#
#   nu publish.nu ingest oxidized-awk --from-ref some-contribution
#
# Then, in your working repo:
#
#   jj git fetch --remote <work-dir>/monorepo.git   # or: git fetch <...> ingest/oxidized-awk
def "main ingest" [
  project: string             # project name, as in projects.nuon
  --from-ref: string = "main" # branch on the published repo to ingest
  --into: string              # local branch to build (default: ingest/<project>)
  --source: string            # monorepo to filter (default: this checkout's origin)
  --work-dir: path            # where the mirror lives
  --owner: string = "overby.me"
  --ssh-user: string
  --ssh-key: path
  --branch: string = "main"   # monorepo branch the work lands on
  --force                     # re-ingest a tip that was ingested before
]: nothing -> nothing {
  check-josh

  let here = ($env.FILE_PWD | default ".")
  let config = ($here | path join "projects.nuon")
  let entry = (open $config | where {|p| $p.name == $project })
  if ($entry | is-empty) {
    error make {msg: $"no project named ($project) in ($config)"}
  }
  let entry = ($entry | first)

  let source = if $source != null { $source } else { ^git remote get-url origin | str trim }
  let work_dir = if $work_dir != null { $work_dir } else {
    ($env.XDG_CACHE_HOME? | default ($env.HOME | path join ".cache")) | path join "tangled-publish"
  }
  let review = if $into != null { $into } else { $"ingest/($project)" }

  if $ssh_key != null {
    # ControlPath=none because a multiplexed connection is already
    # authenticated: with ControlMaster on, ssh reuses the existing socket and
    # never consults -i or IdentitiesOnly, so the knot sees whichever key
    # opened it first and answers "this ssh key doesn't match any key
    # published by the accounts that may push here" no matter what is passed.
    $env.GIT_SSH_COMMAND = $"ssh -i ($ssh_key) -o IdentitiesOnly=yes -o ControlMaster=no -o ControlPath=none -o BatchMode=yes -o StrictHostKeyChecking=accept-new"
  }
  let ssh_login = ($ssh_user | default $owner)
  let remote = $"($ssh_login)@tangled.org:($owner)/($entry.name)"

  let mirror = (sync-mirror $work_dir $source $branch)
  let base = (git-ok $mirror ["rev-parse" $branch])

  # Pull the contribution in as the filtered ref josh will reverse from.
  print $"fetching ($from_ref) from ($remote)"
  git-ok $mirror ["fetch" "--force" $remote $"+refs/heads/($from_ref):refs/josh/($project)"] | ignore
  let incoming = (git-ok $mirror ["rev-parse" $"refs/josh/($project)"])

  # A marker of the last downstream tip taken, so a repeated run says so
  # instead of silently rebuilding the same review branch. rust-lang's
  # josh-sync keeps the same thing in a `rust-version` file; a ref is the
  # equivalent in a bare mirror. It records what was *seen*, not what was
  # merged: that decision is yours and this cannot observe it.
  let marker = $"refs/josh-ingested/($project)"
  let seen = (git-in $mirror ["rev-parse" "--verify" "--quiet" $marker])
  if $seen.exit_code == 0 and ($seen.stdout | str trim) == $incoming and not $force {
    print $"($project) ($from_ref) is already ingested at ($incoming | str substring 0..9)"
    print "  pass --force to build the review branch again"
    return
  }

  # Reverse updates the *input* ref, so point it at a review branch rather
  # than at the monorepo branch itself. Nothing lands on ($branch) here.
  #
  # Derived, and derived `--current`. This read `$entry.filter` until the
  # stored filters were removed from projects.nuon, since when every ingest
  # failed on a missing column; deriving it is what the publish side already
  # does. `--current` is what decides where the contribution lands.
  git-ok $mirror ["update-ref" $"refs/heads/($review)" $base] | ignore
  let filter = (derive-filter $entry --current)
  let out = (do {
    cd $mirror
    ^josh-filter $filter $"refs/heads/($review)" --update $"refs/josh/($project)" --reverse | complete
  })
  if $out.exit_code != 0 {
    error make {msg: $"reverse filter failed: ($out.stderr | str trim)"}
  }

  let tip = (git-ok $mirror ["rev-parse" $"refs/heads/($review)"])
  if $tip == $base {
    print $"nothing to ingest: ($project) ($from_ref) holds no commits the monorepo lacks"
    git-ok $mirror ["update-ref" $marker $incoming] | ignore
    return
  }

  let landed = (git-ok $mirror ["rev-list" "--count" $"($base)..($tip)"])
  print ""
  print $"ingested ($landed) commit\(s\) from ($project) ($from_ref | str trim)"
  print $"  downstream tip: ($incoming | str substring 0..9)"
  print $"  review branch:  ($review) at ($tip | str substring 0..9)"
  print ""
  git-ok $mirror ["log" "--oneline" "--stat" $"($base)..($tip)"] | print
  print ""
  let tip = (run-post-ingest $mirror $entry $review $tip)

  print $"fetch it with:  git fetch ($mirror) ($review)"
  git-ok $mirror ["update-ref" $marker $incoming] | ignore
}

# Run a project's post_ingest commands over the review branch.
#
# The mirror is bare, so there is nothing to run them in: check the branch out
# into a throwaway worktree, run them there, and fold any resulting change
# back onto the branch. rust-lang's josh-sync does the same thing after a pull
# (its example is `cargo fmt`), which only works because it operates inside a
# real checkout.
def run-post-ingest [mirror: path, entry: record, review: string, tip: string]: nothing -> string {
  let hooks = ($entry | get post_ingest? | default [])
  if ($hooks | is-empty) { return $tip }

  let work = ($mirror | path dirname | path join $"ingest-work-($entry.name)")
  rm -rf $work
  git-ok $mirror ["worktree" "add" "--quiet" "--detach" $work $review] | ignore

  # Run inside the project's own directory, not the monorepo root: a
  # formatter invoked at the root would reformat the entire tree rather than
  # the contribution.
  let run_in = ($work | path join $entry.path)
  let run_in = if ($run_in | path exists) { $run_in } else { $work }

  mut current = $tip
  for hook in $hooks {
    print $"  post-ingest [($entry.path)]: ($hook.cmd | str join ' ')"
    let out = (do { cd $run_in; ^($hook.cmd | first) ...($hook.cmd | skip 1) | complete })
    if $out.exit_code != 0 {
      git-in $mirror ["worktree" "remove" "--force" $work] | ignore
      error make {msg: $"post_ingest ($hook.cmd | str join ' ') failed: ($out.stderr | str trim)"}
    }
    let dirty = (git-ok $work ["status" "--porcelain"])
    if ($dirty | is-empty) {
      print "    no change"
      continue
    }
    git-ok $work ["add" "-A"] | ignore
    git-ok $work ["commit" "--quiet" "-m" $hook.message] | ignore
    $current = (git-ok $work ["rev-parse" "HEAD"])
    print $"    committed ($hook.message)"
  }

  # The worktree is detached, so the branch has to be moved onto its result.
  if $current != $tip {
    git-ok $mirror ["update-ref" $"refs/heads/($review)" $current] | ignore
  }
  git-in $mirror ["worktree" "remove" "--force" $work] | ignore
  $current
}
