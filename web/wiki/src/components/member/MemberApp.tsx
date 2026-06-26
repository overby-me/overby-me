import { Stack } from "@mui/material";
import { InvitesFab, InvitesTextField, MembersDataGrid } from "comps";
import type { Node } from "hooks";
import { Suspense } from "react";

const MemberApp = ({ node }: { node: Node }) => {
	if (!node?.id) return null;

	return (
		<>
			<Stack spacing={1}>
				<InvitesTextField node={node} />
				<Suspense fallback={null}>
					<MembersDataGrid node={node} />
				</Suspense>
			</Stack>
			<InvitesFab node={node} />
		</>
	);
};

export default MemberApp;
