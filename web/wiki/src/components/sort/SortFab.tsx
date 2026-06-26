import { Save } from "@mui/icons-material";
import { Fab, Tooltip } from "@mui/material";
import { txMutate } from "core/util";
import type { nodes } from "gql";
import { type Node, useLink } from "hooks";
import { useTranslation } from "react-i18next";

const SortFab = ({
	node,
	elements,
}: {
	node: Node;
	elements: Partial<nodes>[];
}) => {
	const { t } = useTranslation();
	const link = useLink();
	const update = node.useUpdate();

	const handleClick = async () => {
		await Promise.all(
			elements.map(({ id }, index: number) =>
				txMutate(() => update({ id, set: { index } })),
			),
		);
		link.push([]);
	};

	return (
		<Tooltip title={t("sort.saveSorting")}>
			<Fab
				sx={{
					position: "fixed",
					bottom: (t) => t.spacing(9),
					right: (t) => t.spacing(3),
				}}
				color="primary"
				onClick={handleClick}
			>
				<Save />
			</Fab>
		</Tooltip>
	);
};

export default SortFab;
