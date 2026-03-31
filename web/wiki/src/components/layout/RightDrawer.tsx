import { Close } from "@mui/icons-material";
import {
	alpha,
	IconButton,
	Drawer as MuiDrawer,
	Toolbar,
	Typography,
	useMediaQuery,
} from "@mui/material";
import { Box } from "@mui/system";
import { ContentToolbar, MimeAvatarId } from "comps";
import { drawerWidth } from "core/constants";
import { fromId } from "core/path";
import { resolve } from "gql";
import { useNode, usePath, useSession } from "hooks";
import { startTransition, useEffect } from "react";

const Drawer = ({
	open,
	setOpen,
}: {
	open: boolean;
	setOpen: (val: boolean) => void;
}) => {
	const [session, setSession] = useSession();
	const largeScreen = useMediaQuery("(min-width:1200px)");
	const path = usePath();
	const home = path.length === 0;
	const id = session?.nodeId ?? session?.prefix?.id;
	const node = useNode({
		id,
	});

	const contextId = session?.prefix?.id ?? node?.contextId;

	useEffect(() => {
		if (session?.prefix === undefined && !home && id) {
			Promise.all([
				fromId(contextId),
				resolve(({ query }) => {
					const node = query?.node({ id })?.context;
					return {
						id: node?.id,
						name: node?.name ?? "",
						mime: node?.mimeId ?? "",
						key: node?.key,
					};
				}),
			]).then(([path, prefix]) => {
				startTransition(() => {
					setSession({
						prefix: {
							...prefix,
							path,
						},
					});
				});
			});
		}
	}, [session, setSession, id]);

	return (
		<MuiDrawer
			sx={
				largeScreen
					? {
							width: drawerWidth,
							flexShrink: 0,
							"& .MuiDrawer-paper": {
								width: drawerWidth,
								height: `calc(100% - 57px)`,
								boxSizing: "border-box",
							},
						}
					: {
							position: "absolute",
							width: "100%",
							"& .MuiDrawer-paper": {
								width: "100%",
							},
						}
			}
			anchor="right"
			variant={largeScreen ? "permanent" : "persistent"}
			open={open || largeScreen}
			onMouseLeave={() => setOpen(false)}
		>
			<Box
				sx={{
					// Disable scroll (Firefox)
					scrollbarWidth: "none",
					// Disable scroll (Webkit)
					"::-webkit-scrollbar": {
						display: "none",
					},
					overflowY: "auto",
					WebkitOverflowScrolling: "touch",
					height: "calc(100vh - 64px)",
				}}
			>
				<Toolbar
					onClick={() => {
						if (!home) {
							//router.push(`/${(session?.prefix?.path ?? path).join('/')}`);
							setOpen(false);
						}
					}}
					sx={{
						cursor: "pointer",
						ml: largeScreen ? -2 : 0,
						bgcolor: "primary.main",
						"&:hover, &:focus": {
							bgcolor: (t) => alpha(t.palette.primary.main, 0.9),
						},
					}}
				>
					<Box sx={{ flexGrow: 1 }} />
					{id && <MimeAvatarId id={id} />}
					<Typography sx={{ pl: 1 }} color="#fff">
						{node?.name}
					</Typography>
					<Box sx={{ flexGrow: 1 }} />
					{!largeScreen && (
						<IconButton
							sx={{ color: "#fff" }}
							onClick={(e) => {
								e.stopPropagation();
								setOpen(false);
							}}
						>
							<Close />
						</IconButton>
					)}
				</Toolbar>
				<ContentToolbar node={node} />
			</Box>
		</MuiDrawer>
	);
};

export default Drawer;
