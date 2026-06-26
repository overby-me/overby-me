import { Cancel, HowToVote } from "@mui/icons-material";
import {
	Avatar,
	Badge,
	IconButton,
	List,
	ListItemAvatar,
	ListItemButton,
	ListItemSecondaryAction,
	ListItemText,
	Tooltip,
} from "@mui/material";
import { HeaderCard } from "comps";
import { type Node, useLink, useScreen } from "hooks";
import { IconId } from "mime";
import { Fragment, Suspense, startTransition } from "react";
import { useTranslation } from "react-i18next";

const PollListSuspense = ({ node }: { node: Node }) => {
	const { t } = useTranslation();
	const link = useLink();
	const query = node.useQuery();
	const $delete = node.useDelete();
	const polls = query?.children({
		where: { mimeId: { _eq: "vote/poll" } },
	});
	const handleDeletePoll = (id?: string) => () => {
		if (!id) return;
		startTransition(() => $delete({ id }));
	};

	const owner = query?.isContextOwner;

	const card = (
		<HeaderCard
			avatar={
				<Avatar
					sx={{
						bgcolor: "secondary.main",
					}}
				>
					<IconId mimeId="vote/poll" />
				</Avatar>
			}
			title={t("poll.polls")}
		>
			<List>
				{polls?.map(({ id, key, children_aggregate, createdAt }) => (
					<Fragment key={id ?? 0}>
						<ListItemButton onClick={() => link.push([key!])}>
							<Tooltip title={t("vote.voteCount")}>
								<ListItemAvatar>
									<Badge
										color="primary"
										max={1000}
										overlap="circular"
										badgeContent={
											children_aggregate().aggregate?.count() || "0"
										}
									>
										<Avatar
											sx={{
												bgcolor: "secondary.main",
											}}
										>
											<HowToVote />
										</Avatar>
									</Badge>
								</ListItemAvatar>
							</Tooltip>
							<ListItemText
								primary={`${new Date(createdAt!).toLocaleString("da-DK")}`}
							/>
							{owner && (
								<ListItemSecondaryAction>
									<IconButton
										onClick={handleDeletePoll(id)}
										color="primary"
										edge="end"
										size="large"
									>
										<Cancel />
									</IconButton>
								</ListItemSecondaryAction>
							)}
						</ListItemButton>
					</Fragment>
				))}
			</List>
		</HeaderCard>
	);

	return polls?.[0]?.id ? card : null;
};

const PollList = ({ node }: { node: Node }) => {
	const screen = useScreen();

	if (screen) return null;
	return (
		<Suspense fallback={null}>
			<PollListSuspense node={node} />
		</Suspense>
	);
};

export default PollList;
