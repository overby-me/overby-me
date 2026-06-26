import { useAuthenticationStatus } from "@nhost/react";
import { Loader, PathLoader } from "comps";
import { usePathList } from "hooks";
import { Suspense } from "react";

const Path = () => {
	const fullpath = usePathList();
	const { isLoading } = useAuthenticationStatus();

	if (isLoading || fullpath.length === 0) return null;

	// Load node with uuid v4
	if (
		typeof window !== "undefined" &&
		fullpath.length === 1 &&
		fullpath?.[0]?.match(
			/^[0-9A-F]{8}-[0-9A-F]{4}-[4][0-9A-F]{3}-[89AB][0-9A-F]{3}-[0-9A-F]{12}$/i,
		)
	) {
		return (
			<Suspense fallback={null}>
				<Loader id={fullpath[0]} />
			</Suspense>
		);
	}

	// Load node from path
	return (
		<Suspense fallback={null}>
			<PathLoader keys={[]} fullpath={fullpath} />
		</Suspense>
	);
};

export default Path;
