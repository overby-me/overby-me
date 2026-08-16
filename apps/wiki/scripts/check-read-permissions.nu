#!/usr/bin/env nu
# Does a context OWNER read as much as a plain member of that context?
#
# The read rules live in Hasura metadata, in the nhost project, not in this
# repo. `nodes` spells out every way of belonging to a context: you own the
# row, you are a member of its context, its context is public, you were invited
# by email, OR YOU OWN ITS CONTEXT. Five other tables repeat the same list and
# leave that last one out, and then the owner of a context who is not also a
# member of it is an outsider everywhere inside it -- no author chips, no
# relations, no attachments, no permission rows, no faces. That is what the
# Generalsekretær account hit on Landsmøde 2026 (it owns the context and holds
# no membership in it), reported as "cannot see authors".
#
# This checks all five and can put the clause back:
#
#   check-read-permissions.nu <admin-secret-file>            # report
#   check-read-permissions.nu <admin-secret-file> --apply    # report and fix
#
# It is idempotent: a table that already carries the clause is left alone.

const ENDPOINT = "https://pgvhpsenoifywhuxnybq.hasura.eu-central-1.nhost.run/v1/metadata"
const ME = { _eq: "X-Hasura-User-Id" }

# Which tables mirror the `nodes` rule, and where the clause belongs in each
# one's filter. The path is into the `_or` list that holds the sibling ways of
# belonging; the clause is what says "I own this row's context".
const RULES = [
    [table, schema, path, clause];
    ["users", "auth", ["_or"], { memberships: { parent: { owner_id: { _eq: "X-Hasura-User-Id" } } } }]
    ["members", "public", ["_or", "0", "_or"], { parent: { context: { owner_id: { _eq: "X-Hasura-User-Id" } } } }]
    ["permissions", "public", ["context", "_or"], { context: { owner_id: { _eq: "X-Hasura-User-Id" } } }]
    ["relations", "public", ["parent", "_or"], { context: { owner_id: { _eq: "X-Hasura-User-Id" } } }]
    ["files", "storage", ["_or", "1", "nodes", "_or"], { context: { owner_id: { _eq: "X-Hasura-User-Id" } } }]
]

# A list of records describes as `table<...>`, a list of anything else as
# `list<...>`; both are indexed by number.
def indexed [value: any] {
    let kind = ($value | describe)
    ($kind | str starts-with "list") or ($kind | str starts-with "table")
}

# Follow a path of keys, stepping into lists by index. `get` will not do this
# from a variable: handed the string "_or.0._or" it looks for a column with
# dots in its name.
def dig [value: any, keys: list<string>] {
    mut here = $value
    for key in $keys {
        $here = (if (indexed $here) { $here | get ($key | into int) } else { $here | get $key })
    }
    $here
}

# The same walk, rebuilding on the way out.
def plant [value: any, keys: list<string>, leaf: any] {
    if ($keys | is-empty) { return $leaf }
    let key = ($keys | first)
    let rest = ($keys | skip 1)
    if (indexed $value) {
        let at = ($key | into int)
        $value | update $at (plant ($value | get $at) $rest $leaf)
    } else {
        $value | update $key (plant ($value | get $key) $rest $leaf)
    }
}

def metadata [secret: string] {
    let out = (
        http post --content-type application/json --headers { "x-hasura-admin-secret": $secret } $ENDPOINT {
            type: "export_metadata", args: {}
        }
    )
    # The export answers `{resource_version, metadata: {...}}`; older ones answer
    # the metadata itself.
    if "metadata" in ($out | columns) { $out.metadata } else { $out }
}

# The role's select permission for one table, or null if the table is untracked.
def permission-of [meta: record, schema: string, table: string] {
    let tables = ($meta.sources | where name == "default" | get 0.tables)
    let found = ($tables | where {|t| $t.table.name == $table and $t.table.schema == $schema })
    if ($found | is-empty) { return null }
    let perms = ($found | get 0 | get -o select_permissions | default [] | where role == "user")
    if ($perms | is-empty) { null } else { $perms | get 0.permission }
}

def main [
    secretfile: string,
    --apply,
    # Read a saved `export_metadata` answer instead of asking Hasura, so the
    # check can be exercised against a known-bad export without a live project.
    --metadata: string,
] {
    let secret = (open $secretfile | str trim)
    let meta = (if $metadata == null {
        metadata $secret
    } else {
        let saved = (open $metadata)
        if "metadata" in ($saved | columns) { $saved.metadata } else { $saved }
    })

    mut ops = []
    for rule in $RULES {
        let perm = (permission-of $meta $rule.schema $rule.table)
        if $perm == null {
            print $"($rule.schema).($rule.table): NO `user` select permission"
            continue
        }
        let siblings = (dig $perm.filter $rule.path)
        if ($rule.clause in $siblings) {
            print $"($rule.schema).($rule.table): ok"
            continue
        }
        print $"($rule.schema).($rule.table): MISSING the context-owner clause"
        let widened = ($perm | update filter (plant $perm.filter $rule.path ($siblings | append $rule.clause)))
        let t = { name: $rule.table, schema: $rule.schema }
        $ops = ($ops | append [
            { type: "pg_drop_select_permission", args: { source: "default", table: $t, role: "user" } }
            { type: "pg_create_select_permission", args: { source: "default", table: $t, role: "user", permission: $widened } }
        ])
    }

    if ($ops | is-empty) {
        print "every table grants a context owner the read a member of it has."
        return
    }
    if not $apply {
        print $"(($ops | length) // 2) table\(s\) would be changed. Re-run with --apply."
        return
    }
    let out = (
        http post --content-type application/json --allow-errors
            --headers { "x-hasura-admin-secret": $secret } $ENDPOINT { type: "bulk", args: $ops }
    )
    print ($out | to json -r)
}
