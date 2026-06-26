import { Cancel, DoNotTouch, Lock, LockOpen } from "@mui/icons-material";
import {
	Avatar,
	Card,
	CardHeader,
	Collapse,
	IconButton,
	List,
	ListItem,
	ListItemAvatar,
	ListItemButton,
	ListItemSecondaryAction,
	ListItemText,
	Paper,
	Tooltip,
	Typography,
} from "@mui/material";
import { useUserId } from "@nhost/react";
import { avatars, getAvatarName } from "comps";
import { order_by } from "gql";
import { type Node, useScreen } from "hooks";
import { startTransition } from "react";
import { useTranslation } from "react-i18next";
import { TransitionGroup } from "react-transition-group";

const timeString = (time: number) => {
	const sec = String(time % 60);
	const secFmt = sec.length > 1 ? sec : `0${sec}`;
	const min = String(Math.floor(time / 60));
	const minFmt = min.length > 1 ? min : `0${min}`;

	return `${minFmt}:${secFmt}`;
};

const SpeakCard = ({ node, time }: { node: Node; time: number }) => {
	const { t } = useTranslation();
	const screen = useScreen();
	const userId = useUserId();
	const get = node.useSubsGet();
	const $delete = node.useDelete({ refetch: false });
	const speakerlist = get("speakerlist");
	const speakers = speakerlist?.children({
		where: { mimeId: { _eq: "speak/speak" } },
		order_by: [{ data: order_by.desc }, { createdAt: order_by.asc }],
	});

	const handleRemoveSpeak = (id: string) => () => {
		startTransition(() => $delete({ id }));
	};

	return (
		<Card sx={{ m: 0 }}>
			<CardHeader
				title={<Typography variant="h5">{t("speak.speakerList")}</Typography>}
				avatar={
					<Avatar
						sx={{
							bgcolor: (t) =>
								speakerlist?.mutable
									? t.palette.success.main
									: t.palette.error.main,
						}}
					>
						{speakerlist?.mutable ? <LockOpen /> : <Lock />}
					</Avatar>
				}
				action={
					<Paper
						sx={{
							bgcolor: "primary.main",
							pl: 2,
							pr: 2,
							pt: 0.5,
							pb: 0.5,
							borderRadius: "20px",
						}}
					>
						<Typography
							variant="h4"
							sx={{
								color: "common.white",
							}}
						>
							{timeString(time)}
						</Typography>
					</Paper>
				}
			/>
			<List>
				<TransitionGroup>
					{speakers?.map(({ id = "0", name, data, ownerId }) => {
						const avatarData = data();
						const item = (
							<Collapse key={id}>
								<ListItem>
									{avatarData && (
										<Tooltip title={getAvatarName(avatarData)}>
											<ListItemAvatar>
												{avatars[avatarData]?.avatar}
											</ListItemAvatar>
										</Tooltip>
									)}
									<ListItemText
										primary={
											<Typography sx={{ fontSize: "120%" }}>{name}</Typography>
										}
									/>
									{!screen &&
										(userId === ownerId || speakerlist?.isContextOwner) && (
											<ListItemSecondaryAction>
												<IconButton
													onClick={handleRemoveSpeak(id)}
													color="primary"
													edge="end"
													aria-label={t("speak.removeFromList")}
													size="large"
												>
													<Cancel />
												</IconButton>
											</ListItemSecondaryAction>
										)}
								</ListItem>
							</Collapse>
						);
						return id ? item : null;
					})}
					{speakers?.length === 0 && (
						<Collapse key={-1}>
							<ListItemButton>
								<ListItemAvatar>
									<Avatar
										sx={{
											bgcolor: "secondary.main",
										}}
									>
										<DoNotTouch />
									</Avatar>
								</ListItemAvatar>
								<ListItemText primary={t("speak.emptyList")} />
							</ListItemButton>
						</Collapse>
					)}
				</TransitionGroup>
			</List>
		</Card>
	);
};

export default SpeakCard;
