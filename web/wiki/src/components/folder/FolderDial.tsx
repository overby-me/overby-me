import {
	ContentPaste,
	GetApp,
	Lock,
	LockOpen,
	LowPriority,
	SupervisorAccount,
} from "@mui/icons-material";
import { Avatar, SpeedDial, SpeedDialAction, Zoom } from "@mui/material";
import { toHtml } from "core/document";
import { txMutate } from "core/util";
import { order_by, resolve } from "gql";
import { type Node, useLink, useScreen, useSession } from "hooks";
import HTMLtoDOCX from "html-to-docx";
import { getLetter } from "mime";
import { startTransition, useState } from "react";
import { useTranslation } from "react-i18next";

const checkIfSuperParent = async (
	id?: string,
	superParentId?: string,
): Promise<boolean> => {
	if (!(id && superParentId)) return false;
	const { parentId } = await resolve(({ query }) => ({
		parentId: query.node({ id })?.parentId,
	}));
	if (typeof parentId === "object") console.trace(JSON.stringify(parentId));

	return id === superParentId || parentId === superParentId
		? true
		: parentId === null
			? false
			: checkIfSuperParent(parentId!, superParentId);
};

const FolderDial = ({ node }: { node: Node }) => {
	const { t } = useTranslation();
	const screen = useScreen();
	const [session, setSession] = useSession();
	const [open, setOpen] = useState(false);
	const link = useLink();
	const query = node.useQuery();
	const id = query?.id;
	const nodeInsert = node.useInsert();
	const nodeUpdate = node.useUpdate();
	const nodeMembers = node.useMembers();

	if (screen || !query?.isContextOwner) return null;

	const formatContent = async (id: string, level: number): Promise<string> => {
		if (!id) return "";
		const index =
			(
				await resolve(({ query }) =>
					query
						.node({ id })
						?.parent?.children({
							where: {
								_and: [
									{ mutable: { _eq: false } },
									{ mimeId: { _in: ["vote/change", "vote/policy"] } },
								],
							},
							order_by: [{ index: order_by.asc }, { createdAt: order_by.asc }],
						})
						.map(({ id }) => ({ id })),
				)
			)?.findIndex((e) => e.id === id) ?? 0;

		const children = await resolve(({ query }) =>
			query
				.node({ id })
				?.children({
					order_by: [{ index: order_by.asc }],
					where: {
						mimeId: {
							_in: [
								"vote/position",
								"vote/candidate",
								"vote/policy",
								"vote/change",
							],
						},
					},
				})
				.map(({ id }) => id),
		);

		const node = await resolve(({ query }) => {
			const node = query.node({ id });
			if (node)
				return {
					name: node.name,
					data: node.data(),
					mimeId: node.mimeId,
					context: node.mime?.context,
				};
		});

		const members = node?.context
			? []
			: await resolve(({ query }) =>
					query
						.node({ id })
						?.members()
						?.map((m) => m.name ?? m.user?.displayName)
						.join(", "),
				);

		const prefix =
			node?.mimeId === "vote/policy"
				? `${getLetter(index)}: `
				: node?.mimeId === "vote/change"
					? `${index + 1}: `
					: "";

		const formattedMembers = members?.length
			? `<i>${t("folder.proposedBy")}: ${members}</i>`
			: "";

		return `<h${level}>${prefix}${
			node?.name
		}</h${level}>${formattedMembers}${toHtml(node?.data?.content)}${
			children
				? "<br>" +
					(
						await Promise.all(
							children.map((id) => formatContent(id!, level + 1)),
						)
					).join("<br>")
				: ""
		}`;
	};

	const handleExport = async () => {
		if (!id) return;

		const html = await formatContent(id!, 1);

		const blob = await HTMLtoDOCX(html as string, "", {
			table: { row: { cantSplit: true } },
		});

		// Create and evoke link to file
		const blobUrl = URL.createObjectURL(blob as Blob);
		const link = document.createElement("a");
		// eslint-disable-next-line functional/immutable-data
		link.href = blobUrl;
		// eslint-disable-next-line functional/immutable-data
		link.download = `${query?.name}.docx`;
		document.body.appendChild(link);
		link.dispatchEvent(
			new MouseEvent("click", {
				bubbles: true,
				cancelable: true,
				view: window,
			}),
		);
		document.body.removeChild(link);
	};

	const copy = async (copyId?: string | null, parentId?: string | null) => {
		if (!copyId) return;
		const node = await resolve(({ query }) => {
			const {
				name,
				key,
				mimeId,
				data,
				mutable,
				attachable,
				index,
				createdAt,
				owner,
			} =
				query.node({
					id: copyId,
				}) ?? {};
			return {
				name,
				key,
				mimeId: mimeId!,
				mutable,
				attachable,
				index,
				data: data?.(),
				parentId: parentId!,
				createdAt: createdAt!,
				ownerName: owner?.displayName,
			};
		});
		const children = await resolve(({ query }) =>
			query
				.node({ id: copyId })
				?.children()
				.map((child) => child.id),
		);
		const members = await resolve(({ query }) =>
			query
				.members({ where: { parentId: { _eq: copyId } } })
				.map(({ name, nodeId, email }) => ({ name, nodeId, email, parentId })),
		);
		if (node.parentId === null) return;

		const nodeArgs = ["vote/question", "vote/comment"].includes(node.mimeId)
			? { ...node, name: node.ownerName }
			: { ...node, ownerName: undefined };

		const newNode = await txMutate(() => nodeInsert(nodeArgs));
		if (typeof newNode.id !== "string") {
			setSession({ selected: [] });
			return;
		}
		await txMutate(() => nodeMembers.insert({ members, parentId: newNode.id }));

		if (!children) return;
		for (const id of children) {
			await copy(id, newNode.id);
		}

		setSession({ selected: [] });
	};

	const handlePaste = async () => {
		if (
			(
				await Promise.all(
					session?.selected?.map((id) => checkIfSuperParent(node.id, id)) ?? [],
				)
			).some((e) => e)
		) {
			return;
		}
		session?.selected?.map((id) => copy(id, node.id));
	};

	const handleLockChildren = () => {
		startTransition(() =>
			nodeUpdate({ set: { attachable: !query?.attachable } }),
		);
	};

	const handleLockContent = () => {
		startTransition(() => nodeUpdate({ set: { mutable: !query?.mutable } }));
	};

	return (
		<Zoom in>
			<SpeedDial
				ariaLabel={t("folder.manageFolder")}
				sx={{
					position: "fixed",
					bottom: (t) => t.spacing(24),
					right: (t) => t.spacing(3),
				}}
				icon={<SupervisorAccount />}
				onOpen={() => setOpen(true)}
				onClose={() => setOpen(false)}
				open={open}
			>
				<SpeedDialAction
					icon={
						<Avatar sx={{ bgcolor: "primary.main" }}>{<ContentPaste />}</Avatar>
					}
					tooltipTitle={t("common.paste")}
					tooltipOpen
					onClick={handlePaste}
				/>
				<SpeedDialAction
					icon={
						<Avatar sx={{ bgcolor: "primary.main" }}>
							{query?.attachable ? <Lock /> : <LockOpen />}
						</Avatar>
					}
					tooltipTitle={t("folder.lockSubmit", {
						action: t(query?.attachable ? "folder.lock" : "folder.unlock"),
					})}
					tooltipOpen
					onClick={handleLockChildren}
				/>
				<SpeedDialAction
					icon={
						<Avatar sx={{ bgcolor: "primary.main" }}>
							{query?.mutable ? <Lock /> : <LockOpen />}
						</Avatar>
					}
					tooltipTitle={t("folder.lockContent", {
						action: t(query?.mutable ? "folder.lock" : "folder.unlock"),
					})}
					tooltipOpen
					onClick={handleLockContent}
				/>
				<SpeedDialAction
					icon={
						<Avatar sx={{ bgcolor: "primary.main" }}>
							<LowPriority />
						</Avatar>
					}
					tooltipTitle={t("mime.sort")}
					tooltipOpen
					onClick={() => link.push([], "sort")}
				/>
				<SpeedDialAction
					icon={
						<Avatar sx={{ bgcolor: "primary.main" }}>
							<GetApp />
						</Avatar>
					}
					tooltipTitle={t("folder.export")}
					tooltipOpen
					onClick={handleExport}
				/>
			</SpeedDial>
		</Zoom>
	);
};

export default FolderDial;
