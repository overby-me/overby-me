#!/usr/bin/env nu
# Read-only download of every file in the interim's storage, for
# `appview import-files` to file under the ids they already have:
#
#   dump-interim-files.nu <dir>  ->  <dir>/<file id> ...  +  <dir>/manifest.json
#   appview import-files extraction.json <dir>
#
# It only reads: one GraphQL query for the list, then a GET per file. The admin
# secret comes from the environment and is never written anywhere. Like the
# snapshot dump, running it against live data is the owner's step; the committed
# script is the reviewable artifact.
#
# Usage:
#   $env.HASURA_URL = "https://<project>.hasura.<region>.nhost.run/v1/graphql"
#   $env.NHOST_STORAGE_URL = "https://<project>.storage.<region>.nhost.run/v1"
#   $env.HASURA_ADMIN_SECRET = "<secret>"   # never commit this
#   nu scripts/dump-interim-files.nu files
#
# A file already in <dir> at the size storage reports is not fetched again, so a
# run that was cut short is simply run again. `import-files` checks each size
# against the manifest, and refuses a file that came across short.
#
# One file at a time with a pause between (DUMP_PAUSE_MS, default 150): storage
# shares one small project with everything else the interim runs on.

def main [dir: path] {
  let hasura = ($env | get -o HASURA_URL | default "")
  let storage = ($env | get -o NHOST_STORAGE_URL | default "" | str trim --right --char "/")
  let secret = ($env | get -o HASURA_ADMIN_SECRET | default "")
  if ($hasura | is-empty) or ($storage | is-empty) or ($secret | is-empty) {
    print -e "set HASURA_URL, NHOST_STORAGE_URL and HASURA_ADMIN_SECRET (read-only; the secret is never committed)"
    exit 2
  }
  let headers = {x-hasura-admin-secret: $secret}

  let query = "query { files(where: {isUploaded: {_eq: true}}) { id name mimeType size } }"
  let resp = (http post --content-type application/json --headers $headers $hasura ({query: $query} | to json))
  if ($resp | get -o errors | is-not-empty) {
    print -e $"hasura error: ($resp.errors | to json)"
    exit 1
  }
  let files = $resp.data.files

  mkdir $dir
  $files | to json | save --force ($dir | path join "manifest.json")

  let pause = ($env | get -o DUMP_PAUSE_MS | default "150" | into int | into duration --unit ms)
  mut fetched = 0
  mut kept = 0
  for file in $files {
    let target = ($dir | path join $file.id)
    if ($target | path exists) and ((ls $target | get 0.size | into int) == $file.size) {
      $kept += 1
      continue
    }
    http get --raw --headers $headers $"($storage)/files/($file.id)" | save --force --raw $target
    $fetched += 1
    if ($fetched mod 100) == 0 { print -e $"  ($fetched) fetched" }
    sleep $pause
  }
  print -e $"($files | length) files in storage: ($fetched) fetched, ($kept) here already"
}
