#!/usr/bin/env nu
# Let the roles that may rename a member also change their email address.
#
# The bug this fixes: saving the member edit dialog answers
#
#   [UpdateMemberMutation] field 'email' not found in type: 'members_set_input'
#
# Hasura builds `members_set_input` from the columns a role is allowed to
# update, so an `email` missing from that permission is not a column the app can
# name at all, whatever the frontend sends. No code change can reach it; the
# permission is the fix.
#
# A LIVE metadata write against Hasura with the admin secret, read from the
# environment and NEVER committed. It is idempotent: a re-run reports that there
# is nothing left to widen.
#
# What it deliberately does NOT do:
#
#   * invent a row filter. The existing permission's `filter`/`check` decide WHO
#     may update WHICH member, and they are sent back untouched. Only the column
#     list grows, by exactly one entry.
#   * widen every role. Only roles that may already update `name` are touched,
#     because those are the ones holding the edit dialog this bug is about. A
#     role that may only set `accepted` (an invitee accepting their own
#     invitation) must not gain the ability to re-address a member.
#
# Worth knowing before running it: an owner who can change a member's email can
# point an unclaimed invitation at a different address. They can already invite
# anyone to their own context, so this grants no reach they did not have, but it
# is a real change and not a formality.
#
# Usage:
#   $env.HASURA_URL = "https://<project>.hasura.<region>.nhost.run/v1/graphql"
#   $env.HASURA_ADMIN_SECRET = "<secret>"   # never commit this
#   nu scripts/allow-member-email-update.nu

def fail [msg: string] {
    print -e $"allow-member-email-update: ($msg)"
    exit 1
}

def meta [url: string, secret: string, body: any] {
    let resp = (
        try {
            http post --content-type application/json --headers {x-hasura-admin-secret: $secret} $url ($body | to json)
        } catch {|e|
            fail $"metadata call failed: ($e.msg)"
        }
    )
    if ($resp | describe | str starts-with "record") and ($resp | get -o error | is-not-empty) {
        fail $"hasura error: ($resp | to json)"
    }
    $resp
}

# The roles this script is willing to widen, with their current column list.
def widenable [table: any]: nothing -> list {
    $table
    | get -o update_permissions
    | default []
    | each {|p|
        let cols = ($p.permission | get -o columns | default [])
        # `columns: "*"` already includes email; nothing to do for that role.
        if ($cols | describe | str starts-with "list") {
            { role: $p.role, columns: $cols, permission: $p.permission }
        } else {
            null
        }
    }
    | compact
    | where {|p| "name" in $p.columns and "email" not-in $p.columns }
}

def main [] {
    let gql_url = ($env | get -o HASURA_URL | default "")
    let secret = ($env | get -o HASURA_ADMIN_SECRET | default "")
    if ($gql_url | is-empty) or ($secret | is-empty) {
        fail "set HASURA_URL and HASURA_ADMIN_SECRET (the secret is never committed)"
    }
    let url = ($gql_url | str replace "/v1/graphql" "/v1/metadata")

    let exported = (meta $url $secret { type: "export_metadata", args: {} })
    let sources = ($exported | get -o metadata.sources | default [])
    let found = (
        $sources
        | each {|s|
            let t = ($s.tables | where {|t| $t.table.name == "members" } | first?)
            if $t == null { null } else { { source: $s.name, table: $t } }
        }
        | compact
        | first?
    )
    if $found == null { fail "no `members` table in this Hasura's metadata" }

    let targets = (widenable $found.table)
    let all_roles = ($found.table | get -o update_permissions | default [] | get -o role | default [])
    print $"members update permissions: ($all_roles | str join ', ')"
    if ($targets | is-empty) {
        print "nothing to widen: every role that may rename a member may already set their email"
        exit 0
    }

    # Hasura has no "alter permission", so each role is dropped and recreated
    # from its own definition. Sent as one bulk call so the roles do not sit
    # half-updated if a later one is rejected.
    let ops = (
        $targets
        | each {|t|
            print $"  ($t.role): ($t.columns | length) columns -> + email"
            let widened = ($t.permission | upsert columns ($t.columns | append "email"))
            [
                {
                    type: "pg_drop_update_permission"
                    args: { source: $found.source, table: { schema: "public", name: "members" }, role: $t.role }
                }
                {
                    type: "pg_create_update_permission"
                    args: { source: $found.source, table: { schema: "public", name: "members" }, role: $t.role, permission: $widened }
                }
            ]
        }
        | flatten
    )
    meta $url $secret { type: "bulk", args: $ops } | ignore

    # Read it back rather than trusting the write: the point is what Hasura now
    # serves, not what it accepted.
    let after = (meta $url $secret { type: "export_metadata", args: {} })
    let table = (
        $after | get metadata.sources
        | where {|s| $s.name == $found.source } | first
        | get tables | where {|t| $t.table.name == "members" } | first
    )
    let still = (widenable $table)
    if ($still | is-not-empty) {
        fail $"these roles still cannot set email: ($still | get role | str join ', ')"
    }
    print $"done: ($targets | get role | str join ', ') can now update members.email"
}
