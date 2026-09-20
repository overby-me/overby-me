#!/usr/bin/env nu
# Read-only dump of the interim Hasura/Postgres surface into the
# `{ nodes, members, users, permissions }` snapshot the migration extractor consumes
# (crates/migration-extractor). This is the FRONT of the migration pipeline:
#
#   dump-interim-snapshot.nu  ->  snapshot.json  ->  `extract`  ->  extraction.json
#                                                                ->  `appview import`
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
# Each table is read a page at a time by id, with a pause between pages: the
# interim is one small shared project, and a whole table in one answer is the
# heaviest thing it is ever asked for. Every table's page count is held to its
# own aggregate count, so a capped or torn read fails here and not at import.
# A paged read is only a consistent snapshot while writes are frozen, which is
# how the cutover runs it (docs/cutover-runbook.md); a rehearsal can tolerate
# a row that moved mid-dump.
#
# Tuning: DUMP_PAGE (rows per page, default 500), DUMP_PAUSE_MS (default 700).

# POST a GraphQL query with the admin secret and return the `data` object.
def gql [url: string, secret: string, query: string, variables: record] {
  let resp = (
    http post --content-type application/json --headers {x-hasura-admin-secret: $secret} $url ({query: $query, variables: $variables} | to json)
  )
  if ($resp | get -o errors | is-not-empty) {
    print -e $"hasura error: ($resp.errors | to json)"
    exit 1
  }
  $resp.data
}

# Every row of `table`, by ascending id. `query` takes `$after` and `$limit` and
# answers `rows` plus `total`, the count its own filter matches.
def all_rows [url: string, secret: string, table: string, query: string, page: int, pause: duration] {
  mut rows = []
  mut after = "00000000-0000-0000-0000-000000000000"
  mut total = 0
  loop {
    let data = (gql $url $secret $query {after: $after, limit: $page})
    $total = $data.total.aggregate.count
    let got = $data.rows
    if ($got | is-empty) { break }
    $rows = ($rows | append $got)
    $after = ($got | last | get id)
    print -e $"  ($table): ($rows | length) of ($total)"
    if ($got | length) < $page { break }
    sleep $pause
  }
  if ($rows | length) != $total {
    print -e $"($table): read ($rows | length) rows where the table counts ($total); a row moved mid-dump, run it again"
    exit 1
  }
  $rows
}

let url = ($env | get -o HASURA_URL | default "")
let secret = ($env | get -o HASURA_ADMIN_SECRET | default "")
if ($url | is-empty) or ($secret | is-empty) {
  print -e "set HASURA_URL and HASURA_ADMIN_SECRET (read-only admin query; the secret is never committed)"
  exit 2
}

# The exact fields the extractor's InterimNode / InterimMember / InterimUser
# deserialize (camelCase; `claim_token` aliased to the extractor's `claimToken`).
let nodes_q = "query ($after: uuid!, $limit: Int!) { total: nodesAggregate { aggregate { count } } rows: nodes(where: {id: {_gt: $after}}, order_by: {id: asc}, limit: $limit) { id name key path mimeId parentId contextId ownerId data index mutable attachable createdAt updatedAt deleted_at deleted_root } }"
let members_q = "query ($after: uuid!, $limit: Int!) { total: membersAggregate { aggregate { count } } rows: members(where: {id: {_gt: $after}}, order_by: {id: asc}, limit: $limit) { id name email nodeId parentId accepted active owner hidden claimToken: claim_token } }"
# The address and whether the interim VERIFIED it: a person takes their old
# account over by signing in with that address, so an unverified one is not carried.
let users_q = "query ($after: uuid!, $limit: Int!) { total: usersAggregate { aggregate { count } } rows: users(where: {id: {_gt: $after}}, order_by: {id: asc}, limit: $limit) { id displayName avatarUrl email emailVerified } }"
# Only the rows that open a context to everyone: a context is public when it has
# an ACTIVE `public` row granting select, and that row is the setting. Without
# these every context is extracted closed, the public pages included.
let permissions_q = "query ($after: uuid!, $limit: Int!) { total: permissionsAggregate(where: {role: {_eq: \"public\"}}) { aggregate { count } } rows: permissions(where: {_and: [{id: {_gt: $after}}, {role: {_eq: \"public\"}}]}, order_by: {id: asc}, limit: $limit) { id contextId role select active } }"

let page = ($env | get -o DUMP_PAGE | default "500" | into int)
let pause = ($env | get -o DUMP_PAUSE_MS | default "700" | into int | into duration --unit ms)

let nodes = (all_rows $url $secret "nodes" $nodes_q $page $pause)
let members = (all_rows $url $secret "members" $members_q $page $pause)
let users = (all_rows $url $secret "users" $users_q $page $pause)
let permissions = (all_rows $url $secret "permissions" $permissions_q $page $pause | reject id)

print -e $"dumped ($nodes | length) nodes, ($members | length) members, ($users | length) users, ($permissions | length) public permission rows"
{nodes: $nodes, members: $members, users: $users, permissions: $permissions} | to json
