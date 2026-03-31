import {
	Button,
	Dialog,
	DialogActions,
	DialogContent,
	DialogTitle,
	FormControl,
	InputLabel,
	ListItemIcon,
	ListItemText,
	MenuItem,
	Select,
	Stack,
	TextField,
	Typography,
} from "@mui/material";
import { FileUploader } from "comps";
import { getKey } from "core/hooks/useNode";
import { fromId } from "core/path";
import { resolve } from "gql";
import { type Node, useLink, useSession } from "hooks";
import { getName, IconId } from "mime";
import { startTransition, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { v4 as uuid } from "uuid";

const contextPerm = [
	{
		parents: ["vote/poll"],
		active: true,
		delete: false,
		insert: true,
		select: true,
		update: false,
		mimeId: "vote/vote",
		role: "member",
	},
	{
		parents: ["wiki/folder"],
		active: true,
		delete: true,
		insert: true,
		select: true,
		update: true,
		mimeId: "vote/policy",
		role: "member",
	},
	{
		parents: ["vote/position"],
		active: true,
		delete: true,
		insert: true,
		select: true,
		update: true,
		mimeId: "vote/candidate",
		role: "member",
	},
	{
		parents: ["wiki/event", "wiki/folder", "wiki/group"],
		active: true,
		delete: true,
		insert: true,
		select: true,
		update: true,
		mimeId: "wiki/document",
		role: "owner",
	},
	{
		parents: ["vote/policy", "vote/change", "vote/position"],
		active: true,
		delete: true,
		insert: true,
		select: true,
		update: true,
		mimeId: "vote/poll",
		role: "owner",
	},
	{
		parents: ["vote/position", "wiki/file"],
		active: true,
		delete: true,
		insert: true,
		select: true,
		update: true,
		mimeId: "vote/question",
		role: "member",
	},
	{
		parents: ["vote/policy", "vote/change"],
		active: true,
		delete: true,
		insert: true,
		select: true,
		update: true,
		mimeId: "vote/comment",
		role: "member",
	},
	{
		parents: ["speak/list"],
		active: true,
		delete: true,
		insert: true,
		select: true,
		update: true,
		mimeId: "speak/speak",
		role: "member",
	},
	{
		parents: ["vote/policy", "vote/change", "wiki/file"],
		active: true,
		delete: true,
		insert: true,
		select: true,
		update: true,
		mimeId: "vote/change",
		role: "member",
	},
	{
		parents: ["wiki/folder", "wiki/group", "wiki/event"],
		active: true,
		delete: true,
		insert: true,
		select: true,
		update: true,
		mimeId: "wiki/folder",
		role: "owner",
	},
	{
		parents: ["wiki/folder"],
		active: true,
		delete: true,
		insert: true,
		select: true,
		update: true,
		mimeId: "vote/position",
		role: "owner",
	},
	{
		parents: ["wiki/event", "wiki/folder", "wiki/group"],
		active: true,
		delete: true,
		insert: true,
		select: true,
		update: true,
		mimeId: "wiki/file",
		role: "owner",
	},
];

const AddContentDialog = ({
	node,
	open,
	setOpen,
	mimes,
	initTitle,
	redirect,
	app,
	mutable = true,
}: {
	node: Node;
	open: boolean;
	setOpen: (open: boolean) => void;
	mimes: string[];
	initTitle?: string;
	redirect?: boolean;
	app?: string;
	mutable?: boolean;
}) => {
	const { t } = useTranslation();
	const link = useLink();
	const [error, setError] = useState("");
	const [title, setTitle] = useState<string>(initTitle ?? "");
	const [text, setText] = useState<string>("");
	const [mimeId, setMimeId] = useState(mimes?.[0] ?? "");
	const [fileId, setFileId] = useState<string | undefined>();
	const [type, setType] = useState<string | undefined>();
	const [fileName, setFileName] = useState<string>();
	const insert = node.useInsert();
	const update = node.useUpdate();
	const { insert: permInsert } = node.usePermissions();
	const [_, setSession] = useSession();

	const handleSubmit = () => {
		startTransition(async () => {
			try {
				const { id, key } = await insert({
					name: title,
					key: ["vote/question", "vote/comment"].includes(mimeId)
						? uuid()
						: undefined,
					mimeId: mimes.length === 1 ? mimes[0] : mimeId!,
					mutable,
					data:
						mimeId === "wiki/file"
							? { fileId, type }
							: ["vote/question", "vote/comment"].includes(mimeId)
								? { text }
								: undefined,
				});
				if (!key) return;

				if (await resolve(({ query }) => query.mime({ id: mimeId })?.context)) {
					await update({ id: id!, set: { contextId: id, mutable: false } });
					const perms = contextPerm.map((perm) => ({
						...perm,
						contextId: id,
						nodeId: id,
						parents: JSON.stringify(perm.parents)
							.replace("[", "{")
							.replace("]", "}"),
					}));
					await permInsert(perms);

					const prefix = await resolve(({ query }) => {
						const node = query.node({ id: id! });
						return {
							id: node?.id,
							name: node?.name ?? "",
							mime: node?.mimeId ?? "",
							key: node?.key,
						};
					});

					const path = await fromId(id);
					setSession({
						prefix: {
							...prefix,
							path,
						},
					});
				}

				// Reset fields
				setOpen(false);
				setTitle(initTitle ?? "");
				setText("");
				setFileId(undefined);
				setFileName(undefined);

				if (redirect) link.push([key], app);
			} catch (_) {
				setError(t("content.contentNameExists"));
			}
		});
	};

	useEffect(() => {
		if (!node.id) return;
		const checkKeys = async () => {
			const keys = await resolve(({ query }) =>
				query
					.node({ id: node.id! })
					?.children()
					.map((child) => child.key),
			);
			if (keys?.includes(getKey(title) ?? "")) {
				setError(t("content.contentNameExists"));
			}
		};
		checkKeys();
	}, [title, node.id]);

	return (
		<Dialog maxWidth="xs" fullWidth open={open} onClose={() => setOpen(false)}>
			<DialogTitle color="secondary">
				{mimes.length > 1
					? t("content.addContent")
					: t("content.addType", { type: getName(mimes[0]) })}
			</DialogTitle>
			<DialogContent>
				<Stack sx={{ mt: 1 }} spacing={2}>
					{!["vote/question", "vote/comment"].includes(mimeId) && (
						<TextField
							error={!!error}
							helperText={error}
							autoFocus
							required
							label={t("common.title")}
							fullWidth
							value={title}
							onChange={(e) => {
								setTitle(e.target.value);
								setError("");
							}}
							inputProps={{ maxLength: 100 }}
						/>
					)}
					{mimes.length > 1 && (
						<FormControl required fullWidth>
							<InputLabel required>{t("common.type")}</InputLabel>
							<Select
								label={t("common.type")}
								fullWidth
								required
								value={mimeId}
								renderValue={(mimeId) => {
									const mime = mimes.filter((mime) => mime === mimeId)?.[0];
									return mime ? (
										<MenuItem sx={{ m: -1 }} key={mime} value={mime}>
											<ListItemIcon>
												<IconId mimeId={mime} />
											</ListItemIcon>
											<ListItemText>{getName(mime)}</ListItemText>
										</MenuItem>
									) : null;
								}}
								onChange={(e) => setMimeId(e.target.value)}
							>
								{mimes?.map((mime) => (
									<MenuItem key={mime ?? 0} value={mime}>
										<ListItemIcon>
											<IconId mimeId={mime} />
										</ListItemIcon>
										<ListItemText>{getName(mime)}</ListItemText>
									</MenuItem>
								))}
							</Select>
						</FormControl>
					)}
					{["vote/question", "vote/comment"].includes(mimeId) && (
						<TextField
							sx={{ mt: 1 }}
							autoFocus
							required
							label={getName(mimeId)}
							placeholder={t("content.insertPlaceholder", {
								name: getName(mimeId).toLowerCase(),
							})}
							fullWidth
							value={text}
							onChange={(e) => setText(e.target.value)}
						/>
					)}
					{mimeId === "wiki/file" && (
						<>
							<FileUploader
								text={t("content.uploadFile")}
								onNewFile={({ fileId, file }) => {
									setFileId(fileId);
									setType(file.type);
									setFileName(file.name);
									setTitle(file.name.split(".").slice(0, -1).join("."));
								}}
							/>
							{fileName && (
								<Typography sx={{ mt: 1 }}>
									{t("content.fileUploaded", { fileName })}
								</Typography>
							)}
						</>
					)}
				</Stack>
			</DialogContent>
			<DialogActions>
				<Button
					onClick={() => setOpen(false)}
					color="secondary"
					variant="outlined"
				>
					{t("common.cancel")}
				</Button>
				<Button
					disabled={
						(!["vote/question", "vote/comment"].includes(mimeId) && !title) ||
						(mimes.length !== 1 && !mimeId) ||
						(mimeId === "wiki/file" && !fileId) ||
						(["vote/question", "vote/comment"].includes(mimeId) && !text) ||
						!!error
					}
					onClick={handleSubmit}
					color="secondary"
					variant="outlined"
				>
					{t("common.add")}
				</Button>
			</DialogActions>
		</Dialog>
	);
};

export default AddContentDialog;
