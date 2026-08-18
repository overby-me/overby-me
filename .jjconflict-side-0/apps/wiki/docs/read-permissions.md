# Who may read what

The read rules are Hasura row permissions. They live in the nhost project's
metadata, not in this repository, so nothing in a build or a test run checks
them: `scripts/check-read-permissions.nu` is what checks them, and this note is
what says why.

## The ways of belonging

`nodes`, for the `user` role, spells out every way a person may be entitled to
a row:

1. they own it (`owner_id`),
2. they are a member of its context (`context.members.node_id`),
3. its context is public (`contextPublicPermissions`),
4. they were invited to it by email (`members.emailUser`),
5. **they own its context** (`context.owner_id`),
6. they were invited to its context by email (`context.members.emailUser`).

Five other tables repeat that list against their own node and stop at four:
`members`, `relations`, `permissions`, `files` (storage) and `users`. Each of
them left out (5).

## What that cost

A context has an owner, and the owner does not have to be a member of it. The
Generalsekretær account owns Landsmøde 2026 and holds no membership row in it,
so inside its own context it read as an outsider: the resolution pages showed no
author chips, because a membership row on a page in that context matched none of
the four remaining clauses. It was reported as "cannot see authors" on
`/radikal_ungdom/landsmøde_2026/eksterne_resolutioner/forbyd_kørsel_på_danske_strande`,
and the same gap reached the attachments, the relations, the permission rows and
the faces on every page in the context that the owner did not personally create.

Only `nodes` carried clause (5), which is why the page itself opened and only its
parts were missing. That shape — a page that renders with pieces silently absent —
is what a row rule fails as, so it is worth suspecting whenever one reader sees
less of a page than another.

Fixed 2026-08-05 by adding clause (5) to all five, expressed against each table's
own path to a node: `parent.context.owner_id` for `members`, `context.owner_id`
for `relations` and `permissions`, `nodes.context.owner_id` for storage `files`,
and `memberships.parent.owner_id` for `users`. Nothing was removed, and the
clause grants an owner exactly what a plain member of the same context already
had.

## Checking it

```nu
scripts/check-read-permissions.nu <admin-secret-file>            # report
scripts/check-read-permissions.nu <admin-secret-file> --apply    # put it back
scripts/check-read-permissions.nu <secret> --metadata saved.json # check an export
```

It is idempotent, so it is safe to run against production at any time, and
`--metadata` runs the same check over a saved `export_metadata` answer.

## Still open

A signed-in reader sees FEWER author chips on a public page than a signed-out
visitor does. The `public` role may select member rows on a public node; the
`user` role has no such clause, so signing in loses you the chips. It is not
fixed here because the `user` role's member columns include `email`, which the
`public` role's do not, and a row rule cannot vary its columns per clause:
closing the gap the obvious way would put members' email addresses on public
pages for anyone with an account. Closing it properly means either dropping
`email` from the `user` role (the owner roster in `member.rs` reads it) or
moving the roster to a computed field, as `author_name` and `author_avatar`
already do for the header.
