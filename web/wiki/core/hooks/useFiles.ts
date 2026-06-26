import { nhost } from "nhost";
import { startTransition, useEffect, useState } from "react";

const useFiles = ({
	fileIds,
	quality,
	width = 500,
	height,
	image,
}: {
	fileIds?: string[];
	quality?: number;
	width?: number;
	height?: number;
	image?: boolean;
}) => {
	const [files, setFiles] = useState<string[]>([]);

	useEffect(() => {
		setFiles([]);
		const fetch = async () => {
			if (fileIds) {
				const preUrls = await Promise.all(
					fileIds.map((fileId) =>
						fileId
							? nhost.storage.getPublicUrl({
									fileId,
									...(image
										? {
												quality,
												width,
												height,
											}
										: {}),
								})
							: Promise.resolve(null),
					),
				);
				// Wrap the real setter (after the await) — startTransition does not
				// span an `await`, so the previous outer wrapper deferred nothing.
				startTransition(() => setFiles(preUrls.map((preUrl) => preUrl ?? "")));
			}
		};
		fetch();
	}, [JSON.stringify(fileIds)]);

	return files;
};

export default useFiles;
