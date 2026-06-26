import { GroupAdd } from "@mui/icons-material";
import { Fab } from "@mui/material";
import { SheetReader } from "comps";
import { txMutate } from "core/util";
import type { Node } from "hooks";
import { useTranslation } from "react-i18next";

const InvitesFab = ({ node }: { node: Node }) => {
	const { t } = useTranslation();
	const parentId = node.id;
	const nodeMembers = node.useMembers();
	const handleFile = async (
		fileData: { Fornavn: string; Efternavn: string; Email: string }[],
	) => {
		const members = fileData
			.filter((r) => r?.Email)
			.map((r) => ({
				name: `${r.Fornavn} ${r.Efternavn}`,
				email: r?.Email?.toLowerCase(),
				parentId,
			}));
		await txMutate(() => nodeMembers.insert({ members }));
	};

	return (
		// @ts-expect-error: unknown type
		<SheetReader onFileLoaded={handleFile}>
			<Fab
				sx={{
					position: "fixed",
					bottom: (t) => t.spacing(9),
					right: (t) => t.spacing(3),
				}}
				variant="extended"
				color="primary"
				aria-label={t("invite.addAccess")}
				component="span"
			>
				<GroupAdd />
				import
			</Fab>
		</SheetReader>
	);
};

export default InvitesFab;
