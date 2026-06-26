import { Stop } from "@mui/icons-material";
import { Button } from "@mui/material";
import { AdminCard } from "comps";
import type { Node } from "hooks";
import { startTransition } from "react";
import { useTranslation } from "react-i18next";

const PollAdmin = ({ node }: { node: Node }) => {
	const { t } = useTranslation();
	const sub = node.useSubs();
	const update = node.useUpdate();
	const data = sub?.data();

	const voters = sub?.context
		?.permissions({
			where: {
				_and: [
					{ mimeId: { _eq: "vote/vote" } },
					{ insert: { _eq: true } },
					{
						node: {
							members: { active: { _eq: true } },
						},
					},
				],
			},
		})
		.map((perm) => perm.node?.members_aggregate().aggregate?.count())
		.reduce((total, next) => (total ?? 0) + (next ?? 0), 0);

	const handleStopPoll = () => {
		startTransition(() =>
			update({ set: { mutable: false, data: { ...data, voters } } }),
		);
		//await refetch(() =>
		//  node.query
		//    ?.children({ where: { mimeId: { _eq: "vote/vote" } } })
		//    .map((vote) => vote.data)
		//);
	};

	if (!sub?.mutable || !sub?.isContextOwner) return null;

	return (
		<AdminCard title={t("poll.managePoll")}>
			<Button
				size="large"
				color="secondary"
				variant="contained"
				endIcon={<Stop />}
				sx={{ color: "#fff", m: 2 }}
				onClick={handleStopPoll}
			>
				{t("common.stop")}
			</Button>
		</AdminCard>
	);
};

export default PollAdmin;
