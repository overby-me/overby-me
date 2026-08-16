-- 0015: a locked folder or position takes no new content.
--
-- `attachable` is the lock behind the button on a folder and the one on a
-- position: the code calls it "the owner's lock on adding children", and it is
-- in use -- 70 folders, 25 positions and 42 policies are locked in production
-- today. It was never enforced. The insert rule asks whether this person may
-- add this mime in this context, and never whether the thing they are adding it
-- to is closed, so locking a resolution hid the add button and nothing more.
--
-- Checked before writing this, by evaluating the rule rather than guessing: a
-- plain member adding a policy to a locked folder, or a candidature to a locked
-- position, was ALLOWED.
--
-- Two decisions in the clause below:
--
--   * A context OWNER may still add. The lock is theirs, and a chair who closes
--     candidature and then has to enter a late one by hand should not have to
--     unlock the position to do it. This is why it is not simply the poll rule
--     above, which closes a poll to everybody.
--   * Discussion is exempt (`vote/comment`, `vote/reaction`, `vote/question`).
--     A comment is not an addition to what was locked, and stopping new
--     policies on a submitted resolution should not stop people talking about
--     it. There are two such comments in production and they would have been
--     orphaned by a blanket rule.
--
-- The whole function is restated because that is how Postgres takes a change to
-- one: `create or replace`, with the poll clause above untouched.

CREATE OR REPLACE FUNCTION public.insert_with_email_invites(node nodes, hasura_session json)
 RETURNS boolean
 LANGUAGE sql
 STABLE
AS $function$
SELECT EXISTS(
		SELECT 1
		FROM permissions AS permission,
			nodes AS parent,
			nodes AS subnode
		WHERE node.context_id = permission.context_id
			AND node.mime_id = permission.mime_id
			AND permission.active = true
			AND permission.insert = true
		    AND node.parent_id = parent.id
			AND parent.mime_id = ANY(permission.parents)
			AND NOT (parent.mime_id = 'vote/poll' AND parent.mutable IS FALSE)
			-- A parent LOCKED against new children takes none, unless a context
			-- owner is adding it. `attachable` is what the lock button on a folder
			-- and the one on a position already set; until now it only hid the
			-- add control, so a closed candidature or a submitted resolution would
			-- still accept what did not come from those buttons.
			--
			-- Discussion is exempt. A comment or a reaction is not an addition to
			-- what was locked, and locking a resolution to stop new policies
			-- should not also stop people talking about it.
			AND NOT (
				parent.attachable IS FALSE
				AND node.mime_id <> ALL (ARRAY['vote/comment', 'vote/reaction', 'vote/question'])
				AND NOT EXISTS (
					SELECT 1
					FROM members AS ctx_owner
					WHERE ctx_owner.parent_id = node.context_id
						AND ctx_owner.node_id = (hasura_session->>'x-hasura-user-id')::uuid
						AND ctx_owner.owner = true
				)
			)
			AND subnode.id = permission.node_id
			AND EXISTS (
				(
					SELECT 1
					WHERE (
							(
								permission.role = ANY('{"owner", "member"}')
								AND (
									(
										subnode.owner_id = (hasura_session->>'x-hasura-user-id')::uuid
									)
								)
							)
						)
				)
				UNION
				(
					SELECT 1
					FROM members as member
					WHERE (
							permission.role = 'owner'
							AND (
								subnode.id = member.parent_id
								AND (
									member.node_id = (hasura_session->>'x-hasura-user-id')::uuid
									AND member.owner = true
								)
							)
						)
				)
				UNION
				(
					SELECT 1
					FROM members as member
					WHERE (
							permission.role = 'member'
							AND (
								subnode.id = member.parent_id
								AND (
									member.node_id = (hasura_session->>'x-hasura-user-id')::uuid
								)
							)
						)
				)
				UNION
				(
					-- Invited by email, whether or not the invitation was accepted. The
					-- row names someone who has since signed up, matched the way the
					-- nodes select rule matches them (members.email -> auth.users.email).
					-- Deliberate: someone invited to a landsmoede may submit a candidacy
					-- or a resolution without having to accept the invitation first.
					SELECT 1
					FROM members as member
					WHERE (
							permission.role = 'member'
							AND (
								subnode.id = member.parent_id
								AND EXISTS (
									SELECT 1
									FROM auth.users AS invited
									WHERE invited.id = (hasura_session->>'x-hasura-user-id')::uuid
										AND lower(invited.email::text) = lower(member.email::text)
								)
							)
						)
				)
			)
	) $function$
;
