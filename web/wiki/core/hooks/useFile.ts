import { nhost } from "nhost";
import { startTransition, useEffect, useState } from "react";

const useFile = ({
	fileId,
	quality,
	width = 500,
	height,
	image,
}: {
	fileId?: string;
	quality?: number;
	width?: number;
	height?: number;
	image?: boolean;
}) => {
	const [file, setFile] = useState<string | undefined>(undefined);

	useEffect(() => {
		setFile(undefined);
		const fetch = async () => {
			if (fileId) {
				const url = await nhost.storage.getPublicUrl({
					fileId,
					...(image
						? {
								quality,
								width,
								height,
							}
						: {}),
				});
				// Wrap the real setter (after the await) — startTransition does not
				// span an `await`, so the previous outer wrapper deferred nothing.
				startTransition(() => setFile(url));
			}
		};
		fetch();
	}, [fileId]);

	return file;
};

export default useFile;
