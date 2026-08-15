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
#   nu publish.nu --only rust-awk        # one project
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

# Run git and fail loudly, returning trimmed stdout.
def git-ok [repo: path, args: list<string>]: nothing -> string {
  let out = (git-in $repo $args)
  if $out.exit_code != 0 {
    error make {msg: $"git ($args | str join ' ') failed: ($out.stderr | str trim)"}
  }
  $out.stdout | str trim
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
# --ssh-user exists because Tangled push is SSH-only and keyed to an
# account-registered public key. In CI that is the bot account's handle, not
# the owner's. The key is forced with IdentitiesOnly: offered several keys, a
# knot rejects on the first non-matching one and gives up.
def main [
  --source: string            # monorepo to filter (default: this checkout's origin)
  --work-dir: path            # where the mirror clone and josh cache live
  --owner: string = "overby.me" # Tangled account owning the published repos
  --ssh-user: string          # SSH login to push as (default: --owner)
  --ssh-key: path             # private key to push with
  --branch: string = "main"   # branch to read from, and to publish as
  --only: string              # only this project
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
    $env.GIT_SSH_COMMAND = $"ssh -i ($ssh_key) -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=accept-new"
    print $"pushing as ($user)@tangled.org with ($ssh_key)"
  }

  let mirror = (sync-mirror $work_dir $source $branch)
  let input = (git-ok $mirror ["rev-parse" $branch])
  print $"monorepo ($branch) at ($input)"
  print ""

  let ssh_login = ($ssh_user | default $owner)
  let results = (
    $projects | each {|p|
      let remote = $"($ssh_login)@tangled.org:($owner)/($p.name)"
      try {
        let sha = (filter-project $mirror $p.name $p.filter $input)
        let published = (remote-head $mirror $remote $branch)
        let commits = (git-ok $mirror ["rev-list" "--count" $"refs/josh/($p.name)"])

        if $published == $sha {
          {project: $p.name, output: ($sha | str substring 0..9), commits: $commits, action: "skipped"}
        } else if $dry_run {
          {project: $p.name, output: ($sha | str substring 0..9), commits: $commits, action: "would push"}
        } else {
          # Force: rewriting monorepo history rewrites every filtered commit.
          git-ok $mirror ["push" "--force" $remote $"($sha):refs/heads/($branch)"] | ignore
          {project: $p.name, output: ($sha | str substring 0..9), commits: $commits, action: "pushed"}
        }
      } catch {|e|
        {project: $p.name, output: "-", commits: "-", action: $"FAILED: ($e.msg)"}
      }
    }
  )

  print ""
  print $"input ($input | str substring 0..9) -> ($projects | length) projects"
  $results | table | print

  let failed = ($results | where {|r| $r.action | str starts-with "FAILED" })
  if not ($failed | is-empty) {
    # A publish pipeline that fails silently is the failure mode here.
    error make {msg: $"($failed | length) projects failed to publish"}
  }
}
