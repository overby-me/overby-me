import { Add, ContactMail, DoNotDisturb } from "@mui/icons-material";
import {
	Avatar,
	IconButton,
	List,
	ListItem,
	ListItemAvatar,
	ListItemText,
	Tooltip,
} from "@mui/material";
import { useUserEmail, useUserId } from "@nhost/react";
import { HeaderCard, MimeAvatarId } from "comps";
import {
	client,
	type members_set_input,
	order_by,
	resolve,
	useMutation,
	useSubscription,
} from "gql";
import { startTransition } from "react";
import { useTranslation } from "react-i18next";

const ListSuspense = () => {
	const { t } = useTranslation();
	const sub = useSubscription();
	const userId = useUserId();
	const email = useUserEmail();
	const invites = sub
		.members({
			where: {
				_and: [
					{ accepted: { _eq: false } },
					{
						_or: [{ nodeId: { _eq: userId } }, { email: { _eq: email } }],
					},
					{ parent: { mimeId: { _in: ["wiki/group", "wiki/event"] } } },
				],
			},
		})
		.filter((invite) => invite.parent?.id);
	const events = !userId
		? []
		: sub.nodes({
				order_by: [{ createdAt: order_by.desc }],
				where: {
					_and: [
						{ mimeId: { _eq: "wiki/event" } },
						{
							_or: [
								{ ownerId: { _eq: userId } },
								{
									members: {
										_and: [
											{ accepted: { _eq: true } },
											{ nodeId: { _eq: userId } },
										],
									},
								},
							],
						},
					],
				},
			});
	const groups = !userId
		? []
		: sub.nodes({
				order_by: [{ createdAt: order_by.desc }],
				where: {
					_and: [
						{ mimeId: { _eq: "wiki/group" } },
						{
							_or: [
								{ ownerId: { _eq: userId } },
								{
									members: {
										_and: [
											{ accepted: { _eq: true } },
											{ nodeId: { _eq: userId } },
										],
									},
								},
							],
						},
					],
				},
			});

	const [updateMember] = useMutation(
		(mutation, args: { id?: string; set: members_set_input }) => {
			if (!args.id) return;
			mutation.updateMember({
				pk_columns: { id: args.id },
				_set: args.set,
			})?.id;
		},
		{
			refetchQueries: [invites, events, groups],
			awaitRefetchQueries: true,
		},
	);

	const [deleteMember] = useMutation(
		(mutation, args: { id?: string }) => {
			if (args.id === undefined) return;
			mutation.deleteMember({ id: args.id })?.id;
		},
		{
			refetchQueries: [invites, events, groups],
			awaitRefetchQueries: true,
		},
	);

	const [acceptExistingMember] = useMutation(
		(mutation, args: { parentId: string; nodeId: string }) => {
			mutation.updateMembers({
				where: {
					_and: [
						{ parentId: { _eq: args.parentId } },
						{ nodeId: { _eq: args.nodeId } },
					],
				},
				_set: { accepted: true },
			})?.affected_rows;
		},
		{
			refetchQueries: [invites, events, groups],
			awaitRefetchQueries: true,
		},
	);

	const handleAcceptInvite = (id?: string, parentId?: string) => () => {
		startTransition(async () => {
			try {
				await updateMember({
					args: { id, set: { accepted: true, nodeId: userId } },
				});
			} catch (_) {
				await deleteMember({ args: { id } });
				if (parentId && userId) {
					await acceptExistingMember({
						args: { parentId, nodeId: userId },
					});
				}
			}

			// Delete cache
			// eslint-disable-next-line functional/immutable-data
			client.cache.clear();
			await resolve(
				({ query }) =>
					query
						.membersAggregate({
							where: {
								_and: [
									{ accepted: { _eq: false } },
									{
										_or: [
											{ nodeId: { _eq: userId } },
											{ email: { _eq: email } },
										],
									},
								],
							},
						})
						.aggregate?.count(),
				{ cachePolicy: "no-cache" },
			);
		});
	};

	return (
		<List>
			{invites.map(({ id, parent }) => {
				const item = (
					<ListItem key={id ?? 0}>
						{parent?.id && (
							<ListItemAvatar>
								<MimeAvatarId id={parent?.id} />
							</ListItemAvatar>
						)}
						<ListItemText primary={parent?.name} />
						<Tooltip
							title={t("invite.acceptInvitation", { name: parent?.name })}
						>
							<IconButton onClick={handleAcceptInvite(id, parent?.id)}>
								<Add />
							</IconButton>
						</Tooltip>
					</ListItem>
				);

				return id ? item : null;
			})}
			{!invites?.[0]?.id && (
				<ListItem>
					<ListItemAvatar>
						<Avatar
							sx={{
								bgcolor: "secondary.main",
							}}
						>
							<DoNotDisturb />
						</Avatar>
					</ListItemAvatar>
					<ListItemText primary={t("invite.noInvitations")} />
				</ListItem>
			)}
		</List>
	);
};

const InvitesUserList = () => {
	const { t } = useTranslation();
	return (
		<HeaderCard
			avatar={
				<Avatar
					sx={{
						bgcolor: "primary.main",
					}}
				>
					<ContactMail />
				</Avatar>
			}
			title={t("invite.invitations")}
		>
			<ListSuspense />
		</HeaderCard>
	);
};

export default InvitesUserList;
