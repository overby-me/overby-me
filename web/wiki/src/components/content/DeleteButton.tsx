import { Delete } from "@mui/icons-material";
import { Button, Dialog, DialogActions, DialogTitle } from "@mui/material";
import { AutoButton } from "comps";
import { txMutate } from "core/util";
import { type Node, useLink } from "hooks";
import { useState } from "react";
import { useTranslation } from "react-i18next";

const DeleteButton = ({ node }: { node: Node }) => {
	const { t } = useTranslation();
	const [open, setOpen] = useState(false);
	const link = useLink();
	const $delete = node.useDelete();
	const members = node.useMembers();

	const handleDelete = async () => {
		await txMutate(() => members.delete());
		await txMutate(() => $delete());
		link.pop();
	};

	return (
		<>
			<AutoButton
				key="delete"
				text={t("common.delete")}
				icon={<Delete />}
				onClick={() => setOpen(true)}
			/>
			<Dialog open={open} onClose={() => setOpen(false)}>
				<DialogTitle>{t("content.confirmDelete")}</DialogTitle>
				<DialogActions>
					<Button
						endIcon={<Delete />}
						variant="contained"
						color="primary"
						onClick={handleDelete}
					>
						{t("common.delete")}
					</Button>
				</DialogActions>
			</Dialog>
		</>
	);
};

export default DeleteButton;
