import {
	DoNotDisturb,
	ExpandLess,
	ExpandMore,
	LowPriority,
} from "@mui/icons-material";
import {
	Avatar,
	Card,
	CardActions,
	CardHeader,
	Collapse,
	IconButton,
	List,
	ListItemAvatar,
	ListItemButton,
	ListItemSecondaryAction,
	ListItemText,
	Typography,
} from "@mui/material";
import { Stack } from "@mui/system";
import {
	AddChangeButton,
	AutoButton,
	Content,
	MemberChips,
	MimeAvatar,
} from "comps";
import { order_by } from "gql";
import { type Node, useLink, useNode, useScreen } from "hooks";
import { IconId } from "mime";
import { Suspense, useState } from "react";
import { useTranslation } from "react-i18next";
import { TransitionGroup } from "react-transition-group";

const ChildListElement = ({ id, index }: { id: string; index: number }) => {
	const node = useNode({ id });
	const query = node.useQuery();
	const link = useLink();
	const [open, setOpen] = useState(false);

	const item = (
		<>
			<ListItemButton onClick={() => link.push([query?.key ?? ""])}>
				<ListItemAvatar>
					<MimeAvatar mimeId={query?.mimeId} index={index} child />
				</ListItemAvatar>
				<Stack>
					<Typography>{query?.name}</Typography>
					<MemberChips node={node} child />
				</Stack>
				<ListItemSecondaryAction>
					<IconButton
						onClick={(e) => {
							e.stopPropagation();
							setOpen(!open);
						}}
						size="large"
					>
						{open ? <ExpandLess /> : <ExpandMore />}
					</IconButton>
				</ListItemSecondaryAction>
			</ListItemButton>
			<Collapse in={open}>
				<Content node={node} fontSize="100%" />
			</Collapse>
		</>
	);
	return id ? item : null;
};

const ChildListRaw = ({ node }: { node: Node }) => {
	const { t } = useTranslation();
	const children = node.useQuery()?.children({
		where: { mimeId: { _eq: "vote/change" } },
		order_by: [{ index: order_by.asc }],
	});

	return (
		<List>
			<TransitionGroup>
				{children?.map(({ id }, index) => (
					<Collapse key={id ?? 0}>
						{id && (
							<Suspense fallback={null}>
								<ChildListElement id={id} index={index} />
							</Suspense>
						)}
					</Collapse>
				))}
				{children?.length === 0 && (
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
							<ListItemText primary={t("vote.noAmendments")} />
						</ListItemButton>
					</Collapse>
				)}
			</TransitionGroup>
		</List>
	);
};

const ChangeList = ({ node }: { node: Node }) => {
	const { t } = useTranslation();
	const screen = useScreen();
	const link = useLink();
	const query = node.useQuery();

	if (screen) return null;

	return (
		<Card sx={{ m: 0 }}>
			<CardHeader
				title={<Typography>{t("vote.amendments")}</Typography>}
				avatar={
					<Avatar
						sx={{
							bgcolor: "primary.main",
						}}
					>
						<IconId mimeId="vote/change" />
					</Avatar>
				}
				action={
					<CardActions sx={{ p: 0 }}>
						{query?.isContextOwner && (
							<AutoButton
								text={t("mime.sort")}
								icon={<LowPriority />}
								onClick={() => link.push([], "sort")}
							/>
						)}
						<AddChangeButton node={node} />
					</CardActions>
				}
			/>
			<ChildListRaw node={node!} />
		</Card>
	);
};

export default ChangeList;
