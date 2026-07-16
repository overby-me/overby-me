#!/usr/bin/env nu
# Read-only dump of the interim Hasura/Postgres surface into the
# `{ nodes, members, users }` snapshot the migration extractor consumes
# (crates/migration-extractor). This is the FRONT of the migration pipeline:
#
#   dump-interim-snapshot.nu  ->  snapshot.json  ->  `extract`  ->  extraction.json
#                                                                ->  migration-loader
#
# It is READ-ONLY (only GraphQL queries, never a mutation) and PII-free by
# construction only in the sense that it commits NOTHING: the admin secret is
# read from the environment and the row data goes to stdout for a separate,
# owner-approved review step. Running it against live data is owner-gated; the
# committed script is the reviewable artifact.
#
# Usage:
#   $env.HASURA_URL = "https://<project>.hasura.<region>.nhost.run/v1/graphql"
#   $env.HASURA_ADMIN_SECRET = "<secret>"   # never commit this
#   nu scripts/dump-interim-snapshot.nu | save --force snapshot.json
#
# CAUTION: Hasura may impose a default row limit. After running, verify the row
# counts against the census (see docs/) before trusting the snapshot; if a table
# is capped, add keyset pagination here. The cutover runbook's verification gates
# (docs/cutover-runbook.md) are the backstop.

# POST a GraphQL query with the admin secret and return the `data` object.
def gql [url: string, secret: string, query: string] {
  let resp = (
    http post --content-type application/json --headers {x-hasura-admin-secret: $secret} $url ({query: $query} | to json)
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
  print -e "set HASURA_URL and HASURA_ADMIN_SECRET (read-only admin query; the secret is never committed)"
  exit 2
}

# The exact fields the extractor's InterimNode / InterimMember / InterimUser
# deserialize (camelCase; `claim_token` aliased to the extractor's `claimToken`).
let nodes_q = "query { nodes { id name key mimeId parentId contextId ownerId data createdAt } }"
let members_q = "query { members { id name email nodeId parentId accepted active owner claimToken: claim_token } }"
let users_q = "query { users { id displayName avatarUrl } }"

let nodes = (gql $url $secret $nodes_q | get nodes)
let members = (gql $url $secret $members_q | get members)
let users = (gql $url $secret $users_q | get users)

print -e $"dumped ($nodes | length) nodes, ($members | length) members, ($users | length) users"
{nodes: $nodes, members: $members, users: $users} | to json
