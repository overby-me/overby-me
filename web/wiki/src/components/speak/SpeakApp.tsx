import { Grid } from "@mui/material";
import { SpeakAdmin, SpeakCard, SpeakDial } from "comps";
import { type Node, useScreen, useSession } from "hooks";
import { Suspense, useEffect, useState } from "react";

const SpeakApp = ({ node }: { node: Node }) => {
	const screen = useScreen();
	const [session] = useSession();
	const [time, setTime] = useState(0);

	const get = node.useSubsGet();
	const speakerlist = get("speakerlist");

	const data = speakerlist?.data();

	useEffect(() => {
		if (!session?.timeDiff && session?.timeDiff !== 0) return;
		const now = new Date();
		const updatedAt = new Date(data?.updatedAt);
		const sec = Math.floor(
			(data?.time ?? 0) -
				(now.getTime() - updatedAt.getTime() - session?.timeDiff) / 1000,
		);
		setTime(sec >= 0 ? sec : 0);
		const interval = setInterval(() => {
			setTime((time) => (time > 1 ? time - 1 : 0));
		}, 1000);
		return () => clearInterval(interval);
	}, [time, data?.time, data?.updatedAt, session?.timeDiff]);

	return (
		<>
			<Grid container spacing={1}>
				{!screen && (
					<Grid size={{ xs: 12, md: 6 }}>
						<Suspense fallback={null}>
							<SpeakAdmin node={node} time={time} />
						</Suspense>
					</Grid>
				)}
				<Grid size={{ xs: 12, md: !screen ? 6 : 12 }}>
					<Suspense fallback={null}>
						<SpeakCard node={node} time={time} />
					</Suspense>
				</Grid>
			</Grid>
			{!screen && speakerlist?.id && (
				<Suspense fallback={null}>
					<SpeakDial node={node} />
				</Suspense>
			)}
		</>
	);
};

export default SpeakApp;
