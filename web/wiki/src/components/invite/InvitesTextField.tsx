import { Autocomplete, Button, Grid, TextField } from "@mui/material";
import { txMutate } from "core/util";
import { order_by, resolve } from "gql";
import type { Node } from "hooks";
import { startTransition, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

type Option = {
	name?: string;
	email?: string;
	userId?: string;
	id: string;
};

const InvitesTextField = ({ node }: { node: Node }) => {
	const { t } = useTranslation();
	const nodeMembers = node.useMembers();
	const [value, setValue] = useState<Option[]>([]);
	const [options, setOptions] = useState<Option[]>([]);
	const [inputValue, setInputValue] = useState("");

	const handleAddInvites = () => {
		const members = value.map((user) => ({
			name: user.name,
			email: user.email,
			nodeId: user.userId,
			parentId: node.id,
		}));

		void txMutate(() => nodeMembers.insert({ members })).then(() =>
			setValue([]),
		);
	};

	useEffect(() => {
		const fetch = async () => {
			const like = `%${inputValue}%`;
			const users = await resolve(
				({ query: { users } }) =>
					users({
						limit: 10,
						where: {
							displayName: { _ilike: like },
						},
						order_by: [{ displayName: order_by.asc }],
					}).map(({ displayName, id }) => ({
						name: displayName,
						userId: id,
						id,
					})),
				{
					cachePolicy: "no-store",
				},
			);

			const newOptions: Option[] = ([] as Option[]).concat(
				users && inputValue.length > 0 ? users : [],
				value ? value : [],
				inputValue ? [{ name: "N/A", email: inputValue, id: inputValue }] : [],
			);

			// Wrap the real setter (after the await) — startTransition does not span
			// an `await`, so the previous outer wrapper deferred nothing.
			startTransition(() => setOptions(newOptions));
		};
		fetch();
	}, [JSON.stringify(value), inputValue]);

	return (
		<Grid style={{ margin: 1 }} container spacing={2}>
			<Grid size={{ xs: 6 }}>
				<Autocomplete
					multiple
					color="primary"
					noOptionsText={t("common.noMatch")}
					options={options}
					getOptionLabel={(option) => option.email ?? option?.name ?? ""}
					defaultValue={options}
					value={value}
					filterSelectedOptions
					includeInputInList
					autoComplete
					autoHighlight
					onChange={(_, newValue) => {
						setOptions(newValue.concat(options));
						setValue(newValue);
					}}
					onInputChange={(_, newInputValue) => {
						setInputValue(newInputValue);
					}}
					renderInput={(params) => (
						<TextField
							{...params}
							color="primary"
							variant="outlined"
							label={t("invite.invite")}
							placeholder={t("invite.nameOrEmail")}
						/>
					)}
				/>
			</Grid>
			<Grid size={{ xs: 6 }}>
				<Button onClick={handleAddInvites} color="primary" variant="contained">
					{t("common.add")}
				</Button>
			</Grid>
		</Grid>
	);
};

export default InvitesTextField;
