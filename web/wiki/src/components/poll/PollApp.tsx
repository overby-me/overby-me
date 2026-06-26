import { Stack } from "@mui/material";
import { PollAdmin } from "comps";
import { type Node, useScreen } from "hooks";
import { Suspense } from "react";

const PollApp = ({ node }: { node: Node }) => {
	const screen = useScreen();

	return (
		<Stack spacing={1}>
			{!screen && (
				<Suspense fallback={null}>
					<PollAdmin node={node} />
				</Suspense>
			)}
			{/* <PollChartSub node={node} /> */}
		</Stack>
	);
};

export default PollApp;
