#!/usr/bin/env nu
# Idempotently seed the `vote/reaction` permission into every EXISTING context
# that lacks it, so emoji reactions work in old contexts. New contexts get the
# rule automatically from the creation template (`context_permission_objects` in
# src/graphql.rs); this backfills the ones created before the mime existed.
#
# A LIVE write against Hasura with the admin secret, read from the environment
# and NEVER committed. Idempotent: it seeds only the still-missing contexts, so a
# re-run reports "nothing to seed".
#
# Usage:
#   $env.HASURA_URL = "https://<project>.hasura.<region>.nhost.run/v1/graphql"
#   $env.HASURA_ADMIN_SECRET = "<secret>"   # never commit this
#   nu scripts/seed-reaction-permissions.nu

def gql [url: string, secret: string, query: string, variables: any] {
  let resp = (
    http post --content-type application/json --headers {x-hasura-admin-secret: $secret} $url ({query: $query, variables: $variables} | to json)
  )
  if ($resp | get -o errors | is-not-empty) {
    print -e $"hasura error: ($resp.errors | to json)"
    exit 1
  }
  $resp.data
}

let url = ($env | get -o HASURA_URL | default "")
let secret = ($env | get -o HASURA_ADMIN_SECRET | default "")
if ($url | is-empty) or ($secret | is-empty) {
  print -e "set HASURA_URL and HASURA_ADMIN_SECRET (the secret is never committed)"
  exit 2
}

# Contexts where comments work (one comment-permission row per context) are
# exactly where reactions attach.
let ctx = (gql $url $secret 'query { permissions(where: {mimeId: {_eq: "vote/comment"}}, limit: 20000) { contextId } }' {})
let contexts = ($ctx.permissions | get contextId | where {|c| $c != null} | uniq)

let rea = (gql $url $secret 'query { permissions(where: {mimeId: {_eq: "vote/reaction"}}, limit: 20000) { contextId } }' {})
let existing = ($rea.permissions | get contextId | where {|c| $c != null} | uniq)

let missing = ($contexts | where {|c| $c not-in $existing})
print -e $"contexts: ($contexts | length), already seeded: ($existing | length), to seed: ($missing | length)"
if ($missing | is-empty) {
  print -e "nothing to seed"
  exit 0
}

# Mirrors REACTION_PARENTS + the member-role reaction rule in src/graphql.rs.
let parents = ["vote/policy" "vote/change" "wiki/document" "wiki/file" "vote/position" "vote/candidate" "vote/comment"]
let objs = ($missing | each {|c| {
  contextId: $c
  nodeId: $c
  mimeId: "vote/reaction"
  role: "member"
  parents: $parents
  active: true
  insert: true
  select: true
  update: true
  delete: true
}})

let res = (gql $url $secret 'mutation($objs: [permissions_insert_input!]!) { insertPermissions(objects: $objs) { affected_rows } }' {objs: $objs})
print -e $"seeded ($res.insertPermissions.affected_rows) vote/reaction permission rows"
