import { Stack } from "@mui/material";
import {
	CandidateList,
	ContentApp,
	FolderDial,
	PollList,
	QuestionList,
} from "comps";
import type { Node } from "hooks";
import { Suspense } from "react";

const PositionApp = ({ node }: { node: Node }) => (
	<Stack spacing={1}>
		<ContentApp node={node} add />
		<Suspense fallback={null}>
			<CandidateList node={node} />
		</Suspense>
		<Suspense fallback={null}>
			<QuestionList node={node} />
		</Suspense>
		<Suspense fallback={null}>
			<PollList node={node} />
		</Suspense>
		<Suspense fallback={null}>
			<FolderDial node={node} />
		</Suspense>
	</Stack>
);

export default PositionApp;
