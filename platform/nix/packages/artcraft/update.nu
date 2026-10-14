#!/usr/bin/env nu
# Pins ArtCraft apps in sources.json to their latest GitHub releases. With no
# arguments it re-pins every app already listed; name an app to add it.
#
# Hashes come from each release's own SHA256SUMS.txt, so the archives are not
# downloaded here; the first build is what checks them.

const repos = "https://github.com/storytold"

# Must match the names craft.nix fetches.
def assets [app: string, version: string] {
  [
    $"($app)-($version)-linux-x86_64.tar.gz"
    $"($app)-($version)-linux-aarch64.tar.gz"
    $"($app)-($version)-macos-universal.dmg"
    $"($app)-cli-($version)-macos-universal.zip"
  ]
}

def pin [app: string] {
  let tag = http get $"https://api.github.com/repos/storytold/($app)/releases/latest" | get tag_name
  let version = $tag | str replace --regex '^v' ''
  let sums = http get --raw $"($repos)/($app)/releases/download/($tag)/SHA256SUMS.txt"
    | decode utf-8
    | lines
    | parse '{hex}  {name}'
  let hashes = assets $app $version | each {|name|
    let hex = $sums | where name == $name | get hex.0?
    if $hex == null {
      error make {msg: $"($app) ($tag) publishes no ($name)"}
    }
    [$name $"sha256-($hex | decode hex | encode base64)"]
  } | into record
  {version: $version, hashes: $hashes}
}

def main [...apps: string] {
  let file = $env.FILE_PWD | path join sources.json
  let sources = if ($file | path exists) { open $file } else { {} }
  let apps = if ($apps | is-empty) { $sources | columns } else { $apps }
  $apps
  | reduce --fold $sources {|app, acc| $acc | upsert $app (pin $app) }
  | sort
  | to json --tabs 1
  | $"($in)\n"
  | save --force $file
}
