import { Delete, DoNotDisturb, Face, LowPriority } from "@mui/icons-material";
import {
	Avatar,
	Card,
	CardActions,
	CardHeader,
	Chip,
	Collapse,
	IconButton,
	List,
	ListItem,
	ListItemAvatar,
	ListItemButton,
	ListItemSecondaryAction,
	ListItemText,
	Typography,
} from "@mui/material";
import { useUserId } from "@nhost/react";
import { AddQuestionButton, AutoButton } from "comps";
import { order_by } from "gql";
import { type Node, useLink, useScreen } from "hooks";
import { IconId } from "mime";
import { startTransition } from "react";
import { useTranslation } from "react-i18next";
import { TransitionGroup } from "react-transition-group";

const QuestionList = ({ node }: { node: Node }) => {
	const { t } = useTranslation();
	const screen = useScreen();
	const link = useLink();
	const query = node.useQuery();
	const $delete = node.useDelete();
	const userId = useUserId();
	const children = query?.children({
		where: { mimeId: { _eq: "vote/question" } },
		order_by: [{ index: order_by.asc }],
	});

	return (
		<Card sx={{ m: 0 }}>
			<CardHeader
				title={<Typography>{t("vote.questions")}</Typography>}
				avatar={
					<Avatar
						sx={{
							bgcolor: "primary.main",
						}}
					>
						<IconId mimeId="vote/question" />
					</Avatar>
				}
				action={
					!screen && (
						<CardActions sx={{ p: 0 }}>
							{query?.isContextOwner && !screen && (
								<AutoButton
									text={t("mime.sort")}
									icon={<LowPriority />}
									onClick={() => link.push([], "sort")}
								/>
							)}
							<AddQuestionButton node={node} />
						</CardActions>
					)
				}
			/>
			<List>
				<TransitionGroup>
					{children?.map((child, index) => {
						const item = (
							<Collapse key={child?.id ?? 0}>
								<ListItem>
									<ListItemAvatar>
										<Avatar
											sx={{
												bgcolor: "secondary.main",
											}}
										>
											{index + 1}
										</Avatar>
									</ListItemAvatar>
									<ListItemText
										primary={child?.data()?.text}
										secondary={
											<Chip
												key={child?.owner?.id}
												icon={<Face />}
												color="secondary"
												variant="outlined"
												size="small"
												sx={{ mr: 0.5 }}
												label={child?.name ?? child?.owner?.displayName}
											/>
										}
									/>
									{!screen &&
										(child.ownerId === userId || query?.isContextOwner) && (
											<ListItemSecondaryAction>
												<IconButton
													color="primary"
													onClick={() => {
														startTransition(() => $delete({ id: child.id }));
													}}
													size="large"
												>
													<Delete />
												</IconButton>
											</ListItemSecondaryAction>
										)}
								</ListItem>
							</Collapse>
						);
						return child?.id ? item : null;
					})}
					{!children?.[0]?.id && (
						<Collapse key={-1}>
							<ListItemButton>
								<ListItemAvatar>
									<Avatar
										sx={{
											bgcolor: "secondary.main",
										}}
									>
										<DoNotDisturb />
									</Avatar>
								</ListItemAvatar>
								<ListItemText primary={t("vote.noQuestions")} />
							</ListItemButton>
						</Collapse>
					)}
				</TransitionGroup>
			</List>
		</Card>
	);
};

export default QuestionList;
