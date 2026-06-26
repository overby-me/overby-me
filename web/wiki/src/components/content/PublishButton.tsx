import { Publish } from "@mui/icons-material";
import {
	Button,
	Dialog,
	DialogActions,
	DialogContent,
	DialogTitle,
} from "@mui/material";
import { AutoButton } from "comps";
import type { Node } from "hooks";
import { startTransition, useState } from "react";
import { useTranslation } from "react-i18next";

const PublishButton = ({
	node,
	handlePublish,
}: {
	node: Node;
	handlePublish?: () => Promise<void>;
}) => {
	const { t } = useTranslation();
	const [open, setOpen] = useState(false);
	const update = node.useUpdate();
	const query = node.useQuery();

	const handler =
		handlePublish ??
		(() => {
			startTransition(() => update({ set: { mutable: false } }));
		});

	if (!query?.mutable) return null;

	return (
		<>
			<AutoButton
				key="sent"
				text={t("content.submit")}
				icon={<Publish />}
				onClick={() => setOpen(true)}
			/>
			<Dialog open={open} onClose={() => setOpen(false)}>
				<DialogTitle>{t("content.confirmSubmit")}</DialogTitle>
				<DialogContent>{t("content.submitWarning")}</DialogContent>
				<DialogActions>
					<Button
						endIcon={<Publish />}
						variant="contained"
						color="primary"
						onClick={async () => {
							await handler();
							setOpen(false);
						}}
					>
						{t("content.submit")}
					</Button>
				</DialogActions>
			</Dialog>
		</>
	);
};

export default PublishButton;
