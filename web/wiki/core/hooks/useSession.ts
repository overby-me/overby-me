import { createContext, startTransition, useContext, useEffect } from "react";

export type Session = {
	path?: string;
	prefix?: { id?: string; name: string; mime: string; path: string[] };
	user?: { id: string; displayName: string; email: string };
	timeDiff?: number;
	theme?: string;
	screen?: { size: string };
	selected?: string[];
	nodeId?: string | null;
};
type SessionSetter = (props: Session | null) => void;

export const SessionContext = createContext<
	[Session | null, SessionSetter] | null
>(null);

const useSession = (): [Session | null, SessionSetter] => {
	const [session, setCtxSession] = useContext(SessionContext) || [null, null];

	useEffect(() => {
		if (!session && setCtxSession && localStorage.session) {
			startTransition(() => {
				setCtxSession(JSON.parse(localStorage.session));
			});
		}
	}, [session]);

	const setSession: SessionSetter = (props) => {
		const newSession =
			!session || props === null ? props : { ...session, ...props };
		localStorage.setItem("session", JSON.stringify(newSession));
		// Session feeds suspending reads (useNode/useApps via context). Mark the
		// context update as non-urgent so a resulting re-suspend keeps the old UI
		// visible instead of throwing React #426 ("suspended on synchronous input").
		if (setCtxSession) startTransition(() => setCtxSession(newSession));
	};

	return [session, setSession];
};

export default useSession;
