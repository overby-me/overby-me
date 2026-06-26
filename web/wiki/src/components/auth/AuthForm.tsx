import { Email, HowToReg, LockReset, Login } from "@mui/icons-material";
import {
	Avatar,
	Box,
	Button,
	CircularProgress,
	Container,
	Stack,
	TextField,
	Typography,
} from "@mui/material";
import { useAuthenticationStatus } from "@nhost/react";
import { client } from "gql";
import { useSession } from "hooks";
import { nhost } from "nhost";
import {
	type ChangeEventHandler,
	type FormEvent,
	startTransition,
	useEffect,
	useState,
} from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

type Mode = "login" | "register" | "reset-password" | "set-password";

const LoginForm = ({ mode }: { mode: Mode }) => {
	const { t } = useTranslation();
	const navigate = useNavigate();
	const { isAuthenticated } = useAuthenticationStatus();
	const [_, setSession] = useSession();
	const [loading, setLoading] = useState(false);
	const [name, setName] = useState("");
	const [email, setEmail] = useState("");
	const [password, setPassword] = useState("");
	const [passwordRepeat, setPasswordRepeat] = useState("");
	const [errorName, setNameError] = useState("");
	const [errorEmail, setEmailError] = useState("");
	const [errorPassword, setPasswordError] = useState("");
	const [errorPasswordRepeat, setPasswordRepeatError] = useState("");

	useEffect(() => {
		if (
			["login", "register", "reset-password"].includes(mode) &&
			!loading &&
			isAuthenticated
		) {
			startTransition(() => navigate("/"));
		}
	}, [isAuthenticated, loading, mode, navigate]);

	const onNameChange: ChangeEventHandler<HTMLInputElement> = (e) => {
		const name = e.target.value;
		setName(name);
		if (name) {
			setNameError("");
		} else {
			setNameError(t("auth.missingName"));
		}
	};

	const onEmailChange: ChangeEventHandler<HTMLInputElement> = (e) => {
		const email = e.target.value;
		setEmail(email);
		if (email) {
			setEmailError("");
			setPasswordError("");
		} else {
			setEmailError(t("auth.missingEmail"));
		}
	};

	const onPasswordChange: ChangeEventHandler<HTMLInputElement> = (e) => {
		const password = e.target.value;
		setPassword(password);
		if (password === passwordRepeat || passwordRepeat === "") {
			setEmailError("");
			setPasswordError("");
			setPasswordRepeatError("");
		} else {
			setPasswordError(t("auth.passwordMismatch"));
		}
	};

	const onPasswordRepeatChange: ChangeEventHandler<HTMLInputElement> = (e) => {
		const passwordRepeat = e.target.value;
		setPasswordRepeat(passwordRepeat);
		if (password === passwordRepeat || passwordRepeat === "") {
			setPasswordError("");
			setPasswordRepeatError("");
		} else {
			setPasswordRepeatError(t("auth.passwordMismatch"));
		}
	};

	const onLogin = async () => {
		if (email === "") {
			setEmailError(t("auth.missingEmail"));
			return;
		}
		if (password === "") {
			setPasswordError(t("auth.missingPassword"));
			return;
		}
		const { error } = await nhost.auth.signIn({
			email: email.toLowerCase(),
			password,
		});
		if (error) {
			// Already logged-in
			if ([100].includes(error.status)) {
				startTransition(() => navigate("/"));
				return;
			}

			if (error.error === "unverified-user") {
				nhost.auth.sendVerificationEmail({ email });
				setEmailError(t("auth.emailNotVerified"));
				setLoading(false);
				return;
			}
			setEmailError(t("auth.wrongCredentials"));
			setPasswordError(t("auth.wrongCredentials"));
			setLoading(false);
			return;
		}

		// Set up
		setSession({
			timeDiff: undefined,
		});

		// Delete cache
		// eslint-disable-next-line functional/immutable-data
		client.cache.clear();

		// cache.clear() invalidates every query, so the route we return to will
		// re-read cold and suspend; defer the navigation so it loads without a
		// synchronous fallback flash (React #426).
		startTransition(() => navigate(-1));
	};

	const onRegister = async () => {
		if (name === "") {
			setNameError(t("auth.missingName"));
			return;
		}
		if (email === "") {
			setEmailError(t("auth.missingEmail"));
			return;
		}
		if (password === "") {
			setPasswordError(t("auth.missingPassword"));
			return;
		}
		if (passwordRepeat === "") {
			setPasswordRepeatError(t("auth.repeatPassword"));
			return;
		}
		const { error } = await nhost.auth.signUp({
			email: email.toLowerCase(),
			password,
			options: { displayName: name },
		});
		if (error) {
			switch (error.error) {
				case "invalid-email":
					setEmailError(t("auth.invalidEmail"));
					break;
				case "email-already-in-use":
					setEmailError(t("auth.emailAlreadyInUse"));
					break;
				default:
					setEmailError(error?.message);
			}
			return;
		}

		navigate("/user/unverified");
	};

	const onSetPassword = async () => {
		if (password === "") {
			setPasswordError(t("auth.missingPassword"));
			return;
		}
		if (passwordRepeat === "") {
			setPasswordRepeatError(t("auth.repeatPassword"));
			return;
		}
		const { error } = await nhost.auth.changePassword({
			newPassword: password,
		});

		if (error) {
			setPasswordError(error.message);
			return;
		}

		startTransition(() => navigate("/"));
	};

	const onSendResetEmail = async () => {
		if (email === "") {
			setEmailError(t("auth.missingEmail"));
			return;
		}
		const { error } = await nhost.auth.resetPassword({
			email: email.toLowerCase(),
		});
		if (error) {
			switch (error.error) {
				case "invalid-email":
					setEmailError(t("auth.invalidEmail"));
					break;
				case "user-not-found":
					setEmailError(t("auth.userNotFound"));
					break;
				default:
					setEmailError(error.message);
			}
			return;
		}
		navigate("/user/set-password");
	};

	const handleSubmit = async (e: FormEvent) => {
		e.preventDefault();
		setLoading(true);

		switch (mode) {
			case "login":
				await onLogin();
				break;
			case "register":
				await onRegister();
				break;
			case "set-password":
				await onSetPassword();
				break;
			case "reset-password":
				await onSendResetEmail();
				break;
		}
		setLoading(false);
	};

	const icon =
		mode === "login" ? (
			<Login />
		) : mode === "register" ? (
			<HowToReg />
		) : mode === "reset-password" ? (
			<Email />
		) : (
			<LockReset />
		);

	const text =
		mode === "login"
			? t("auth.login")
			: mode === "register"
				? t("auth.register")
				: mode === "reset-password"
					? t("auth.resetPassword")
					: t("auth.setPassword");

	return (
		<Container sx={{ padding: 3 }} maxWidth="xs">
			<form onSubmit={handleSubmit}>
				<Stack spacing={2} alignItems="center">
					<Avatar sx={{ bgcolor: "primary.main" }}>{icon}</Avatar>
					<Typography variant="h5">{text}</Typography>
					{mode === "register" && (
						<TextField
							fullWidth
							error={!!errorName}
							helperText={errorName}
							label={t("auth.fullName")}
							name="fullname"
							variant="outlined"
							onChange={onNameChange}
						/>
					)}
					{mode !== "set-password" && (
						<TextField
							fullWidth
							error={!!errorEmail}
							helperText={errorEmail}
							label={t("auth.email")}
							autoComplete="username"
							name="email"
							variant="outlined"
							onChange={onEmailChange}
						/>
					)}
					{mode !== "reset-password" && (
						<TextField
							fullWidth
							error={!!errorPassword}
							helperText={errorPassword}
							label={
								mode === "set-password"
									? t("auth.newPassword")
									: t("auth.password")
							}
							autoComplete="current-password"
							name="password"
							type="password"
							variant="outlined"
							onChange={onPasswordChange}
						/>
					)}
					{["register", "set-password"].includes(mode) && (
						<TextField
							fullWidth
							error={!!errorPasswordRepeat}
							helperText={errorPasswordRepeat}
							label={t("auth.repeatPassword")}
							name="password"
							type="password"
							variant="outlined"
							onChange={onPasswordRepeatChange}
						/>
					)}
					<Box sx={{ position: "relative", width: "100%" }}>
						<Button
							color="primary"
							fullWidth
							type="submit"
							variant="contained"
							startIcon={icon}
							disabled={
								loading ||
								!!(
									errorName ||
									errorEmail ||
									errorPassword ||
									errorPasswordRepeat
								)
							}
						>
							{text}
						</Button>

						{loading && (
							<CircularProgress
								size={24}
								sx={{
									position: "absolute",
									top: "50%",
									left: "50%",
									marginTop: "-12px",
									marginLeft: "-12px",
								}}
							/>
						)}
					</Box>
					{["login"].includes(mode) && (
						<Button
							color="secondary"
							fullWidth
							variant="contained"
							startIcon={<HowToReg />}
							onClick={() => navigate("/user/register")}
						>
							{t("auth.register")}
						</Button>
					)}
					{["login"].includes(mode) && (
						<Button
							color="secondary"
							fullWidth
							variant="contained"
							startIcon={<Email />}
							onClick={() => navigate("/user/reset-password")}
						>
							{t("auth.resetPassword")}
						</Button>
					)}
				</Stack>
			</form>
		</Container>
	);
};

export default LoginForm;
