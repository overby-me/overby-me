use std::collections::HashMap;

use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub enum Lang {
    En,
    Da,
}

impl Lang {
    pub fn code(&self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Da => "da",
        }
    }

    /// BCP-47 locale for browser date/number formatting (`toLocaleString`).
    /// English uses the European (24-hour, day-first) form to match this app's
    /// Danish audience.
    pub fn locale(&self) -> &'static str {
        match self {
            Lang::En => "en-GB",
            Lang::Da => "da-DK",
        }
    }
}

/// The current UI language's locale for `toLocaleString`-style formatting.
pub fn current_locale() -> &'static str {
    LANG.read().locale()
}

pub static LANG: GlobalSignal<Lang> = Signal::global(|| Lang::En);

pub fn use_lang() -> Signal<Lang> {
    LANG.signal()
}

/// Translation function — looks up key in nested translation map
pub fn t(key: &str) -> String {
    let lang = LANG.read();
    let translations = match *lang {
        Lang::En => en_translations(),
        Lang::Da => da_translations(),
    };

    lookup_key(&translations, key).unwrap_or_else(|| key.to_string())
}

/// Translation with interpolation: t_with("layout.greeting", &[("name", "Niclas")])
pub fn t_with(key: &str, params: &[(&str, &str)]) -> String {
    let mut result = t(key);
    for (k, v) in params {
        result = result.replace(&format!("{{{{{k}}}}}"), v);
    }
    result
}

fn lookup_key(map: &HashMap<String, serde_json::Value>, key: &str) -> Option<String> {
    let parts: Vec<&str> = key.splitn(2, '.').collect();
    if parts.len() == 2 {
        if let Some(serde_json::Value::Object(inner)) = map.get(parts[0]) {
            if let Some(serde_json::Value::String(s)) = inner.get(parts[1]) {
                return Some(s.clone());
            }
        }
    }
    None
}

fn en_translations() -> HashMap<String, serde_json::Value> {
    serde_json::from_str(EN_JSON).unwrap_or_default()
}

fn da_translations() -> HashMap<String, serde_json::Value> {
    serde_json::from_str(DA_JSON).unwrap_or_default()
}

const EN_JSON: &str = r#"{
    "editor": {
        "style": "Style",
        "paragraph": "Paragraph",
        "headingOne": "Heading 1",
        "headingTwo": "Heading 2",
        "headingThree": "Heading 3",
        "headingFour": "Heading 4",
        "headingFive": "Heading 5",
        "headingSix": "Heading 6",
        "blockQuote": "Quote",
        "blockPre": "Preformatted",
        "bold": "Bold",
        "italic": "Italic",
        "underline": "Underline",
        "strikethrough": "Strikethrough",
        "code": "Code",
        "bulletedList": "Bulleted list",
        "numberedList": "Numbered list",
        "alignLeft": "Align left",
        "alignCenter": "Align center",
        "alignRight": "Align right",
        "alignJustify": "Justify",
        "undo": "Undo",
        "redo": "Redo",
        "link": "Link",
        "linkUrl": "Link URL",
        "addLink": "Apply",
        "removeLink": "Remove link"
    },
    "common": {
        "add": "Add",
        "cancel": "Cancel",
        "delete": "Delete",
        "edit": "Edit",
        "save": "Save",
        "previous": "Previous",
        "next": "Next",
        "tools": "Tools",
        "apps": "Apps",
        "search": "Search",
        "searchInSection": "Search in this section",
        "searchEverywhere": "Search everywhere",
        "close": "Close",
        "home": "Home",
        "menu": "Menu",
        "send": "Send",
        "expand": "Expand",
        "remove": "Remove",
        "noContent": "No content",
        "noMatch": "No match",
        "noResult": "No result",
        "loading": "Loading...",
        "title": "Title",
        "type": "Type",
        "paste": "Paste",
        "members": "Members",
        "stop": "Stop",
        "logIn": "Log in",
        "register": "Register",
        "unknown": "Unknown",
        "download": "Download",
        "gridView": "Grid view",
        "listView": "List view",
        "items": "Items"
    },
    "auth": {
        "login": "Log In",
        "register": "Register",
        "resetPassword": "Reset Password",
        "setPassword": "Set Password",
        "missingName": "Name required",
        "missingEmail": "Email required",
        "missingPassword": "Password required",
        "repeatPassword": "Repeat Password",
        "fullName": "Full name",
        "email": "Email",
        "password": "Password",
        "newPassword": "New password",
        "emailNotVerified": "Email not verified. Check your inbox. Also check spam.",
        "resendVerification": "Resend verification email",
        "verificationResent": "Verification email sent.",
        "invalidEmail": "Invalid email",
        "wrongCredentials": "Wrong email or password",
        "passwordMismatch": "Passwords do not match",
        "emailAlreadyInUse": "Email is already in use",
        "userNotFound": "No user exists with this email",
        "logout": "Log out",
        "verifyEmail": "Verify your email",
        "verificationEmailSent": "You should have received a verification email.",
        "useToActivate": "Use it to activate your account.",
        "checkSpam": "Check if the email ended up in spam.",
        "checkEmail": "Check your email",
        "passwordResetSent": "You should have received an email.",
        "useToResetPassword": "Use it to reset your password."
    },
    "content": {
        "addContent": "Add content",
        "addType": "Add {{type}}",
        "confirmDelete": "Confirm Deletion",
        "confirmSubmit": "Confirm Submission",
        "submitWarning": "Once you have submitted, it is no longer possible to edit.",
        "submit": "Submit",
        "authors": "Authors",
        "addAuthor": "Add Author",
        "addAtLeastOneAuthor": "Add at least 1 author",
        "uploadImage": "Upload Image",
        "uploadFile": "Upload File",
        "chooseFile": "Choose a file",
        "contentNameExists": "Content with this name already exists",
        "imageAlt": "Content image",
        "tableOfContents": "Table of contents"
    },
    "layout": {
        "welcomeTitle": "Welcome to RadikalWiki",
        "loginOrRegister": "Log in or register.",
        "rememberEmail": "Remember to use the email you registered with at RU.",
        "greeting": "Hello {{name}}!",
        "acceptInvitations": "Please accept your invitations to groups and events.",
        "noInvitationsHint": "If no invitations appear, you most likely used a different email than the one registered with Radikal Ungdom.",
        "groups": "Groups",
        "events": "Events",
        "memberships": "Memberships",
        "content": "Content",
        "noGroups": "No groups",
        "noEvents": "No events",
        "noMemberships": "No memberships",
        "currentItem": "Current Item",
        "exitSearch": "Exit Search Field",
        "notSubmitted": "Not submitted",
        "dark": "Dark",
        "light": "Light",
        "userMenu": "User menu",
        "newestContent": "Newest content",
        "darkMode": "Dark mode",
        "themeColor": "Theme color",
        "primaryColor": "Primary",
        "accentColor": "Accent",
        "customColor": "Custom color"
    },
    "error": {
        "somethingWentWrong": "Something went wrong!",
        "sendMessage": "Please send the following message to"
    },
    "folder": {
        "manageFolder": "Manage folder",
        "proposedBy": "Proposed by",
        "export": "Export",
        "copy": "Copy",
        "paste": "Paste here",
        "lock": "Lock (block adding content)",
        "unlock": "Unlock (allow adding content)"
    },
    "node": {
        "documentUnavailable": "The document is not available",
        "notFoundOrNoAccess": "This may be because the document does not exist, or you do not have access to it.",
        "maybeLoginForAccess": "You may be able to access the document by logging in:"
    },
    "vote": {
        "newAmendment": "New Amendment",
        "newComment": "Write a comment…",
        "comments": "Comments",
        "reply": "Reply",
        "replies": "replies",
        "question": "Question",
        "noAmendments": "No amendments",
        "noComments": "No comments yet",
        "noQuestions": "No questions",
        "amendments": "Amendments",
        "questions": "Questions",
        "candidates": "Candidates",
        "hasVotingRight": "You have voting rights",
        "noVotingRight": "You do not have voting rights",
        "hasNotVoted": "You have not voted",
        "hasVoted": "You have voted",
        "updateStatus": "Update status",
        "noVoteNow": "No vote right now",
        "castVote": "Vote",
        "voteCount": "Number of votes"
    },
    "perm": {
        "type": "Type",
        "role": "Role",
        "insert": "Create",
        "select": "View",
        "delete": "Delete",
        "active": "Active"
    },
    "admin": {
        "poll": "Poll",
        "results": "Results",
        "votes": "Votes"
    },
    "speak": {
        "manageSpeakerList": "Manage Speaker List",
        "joinSpeakerList": "Join the speaker list",
        "open": "Open",
        "close": "Close",
        "talk": "Talk",
        "question": "Question",
        "clarify": "Clarify",
        "misunderstood": "Misunderstood",
        "procedure": "Procedure",
        "speakerList": "Speaker List",
        "removeFromList": "Remove from speaker list",
        "emptyList": "The speaker list is empty",
        "clear": "Clear",
        "start": "Start",
        "stop": "Stop",
        "speakingTime": "Speaking time (s)",
        "speakingNow": "Speaking now",
        "yourTurn": "Your turn to speak",
        "yourTurnBody": "You are the current speaker.",
        "next": "Next",
        "getReady": "Get ready — you're on deck",
        "moveUp": "Move to top",
        "moveDown": "Move to bottom"
    },
    "poll": {
        "managePoll": "Manage Poll",
        "hideResult": "Hide result",
        "newPoll": "New poll",
        "voteRange": "Number of votes",
        "start": "Start",
        "stopPoll": "Stop poll",
        "resultsHidden": "Results hidden"
    },
    "redirect": {
        "forwarding": "Forwarding you to:",
        "noTarget": "No redirect target set yet.",
        "targetUrl": "Target URL"
    },
    "social": {
        "empty": "No posts yet.",
        "query": "Search term",
        "poweredBy": "Live from Bluesky"
    },
    "profile": {
        "memberships": "Your groups and events",
        "signedInAs": "Signed in as",
        "userId": "User ID"
    },
    "program": {
        "empty": "No programme items yet."
    },
    "parent": {
        "title": "Missing parents",
        "description": "Nodes that have lost their parent (orphans).",
        "none": "No orphaned nodes."
    },
    "sort": {
        "saveSorting": "Save sorting"
    },
    "invite": {
        "addAccess": "Add access",
        "noInvitations": "No invitations",
        "invitations": "Invitations",
        "invite": "Invite",
        "nameOrEmail": "Name or Email",
        "acceptInvitation": "Accept invitation to {{name}}",
        "importRoster": "Import roster (.xlsx: Fornavn, Efternavn, Email)",
        "imported": "Imported {{count}} members",
        "noRosterRows": "No rows with an email found in the file"
    },
    "member": {
        "name": "Name",
        "email": "Email",
        "hidden": "Hidden",
        "hide": "Hide member",
        "show": "Show member",
        "owner": "Owner",
        "active": "Active",
        "actions": "Actions",
        "author": "Author",
        "promote": "Make owner",
        "demote": "Remove owner",
        "activate": "Mark active",
        "deactivate": "Mark inactive",
        "remove": "Remove member",
        "edit": "Edit member",
        "confirmRemove": "Remove this member?",
        "export": "Export participants (CSV)",
        "status": "Status",
        "filterAll": "All",
        "search": "Search members"
    },
    "mime": {
        "group": "Group",
        "event": "Event",
        "folder": "Folder",
        "document": "Document",
        "file": "File",
        "person": "Person",
        "policy": "Policy",
        "position": "Position",
        "amendment": "Amendment",
        "candidate": "Candidacy",
        "speakerList": "Speaker List",
        "editor": "Edit",
        "sort": "Sort",
        "speak": "Speak",
        "vote": "Vote",
        "members": "Members",
        "map": "Map",
        "screen": "Screen",
        "admin": "Results",
        "permissions": "Permissions",
        "graph": "Graph",
        "program": "Programme",
        "profile": "Profile",
        "social": "Social wall",
        "parent": "Missing parents",
        "unknown": "Unknown"
    }
}"#;

const DA_JSON: &str = r#"{
    "editor": {
        "style": "Stil",
        "paragraph": "Afsnit",
        "headingOne": "Overskrift 1",
        "headingTwo": "Overskrift 2",
        "headingThree": "Overskrift 3",
        "headingFour": "Overskrift 4",
        "headingFive": "Overskrift 5",
        "headingSix": "Overskrift 6",
        "blockQuote": "Citat",
        "blockPre": "Præformateret",
        "bold": "Fed",
        "italic": "Kursiv",
        "underline": "Understreget",
        "strikethrough": "Gennemstreget",
        "code": "Kode",
        "bulletedList": "Punktliste",
        "numberedList": "Nummereret liste",
        "alignLeft": "Venstrejuster",
        "alignCenter": "Centrer",
        "alignRight": "Højrejuster",
        "alignJustify": "Lige margener",
        "undo": "Fortryd",
        "redo": "Gentag",
        "link": "Link",
        "linkUrl": "Link-URL",
        "addLink": "Anvend",
        "removeLink": "Fjern link"
    },
    "common": {
        "add": "Tilf\u00f8j",
        "cancel": "Annuller",
        "delete": "Slet",
        "edit": "Rediger",
        "save": "Gem",
        "previous": "Forrige",
        "next": "N\u00e6ste",
        "tools": "V\u00e6rkt\u00f8jer",
        "apps": "Apps",
        "search": "S\u00f8g",
        "searchInSection": "S\u00f8g i denne sektion",
        "searchEverywhere": "S\u00f8g overalt",
        "close": "Luk",
        "home": "Hjem",
        "menu": "Menu",
        "send": "Send",
        "expand": "Udvid",
        "remove": "Fjern",
        "noContent": "Intet indhold",
        "noMatch": "Intet match",
        "noResult": "Intet resultat",
        "loading": "Indl\u00e6ser...",
        "title": "Titel",
        "type": "Type",
        "paste": "Inds\u00e6t",
        "members": "Medlemmer",
        "stop": "Stop",
        "logIn": "Log ind",
        "register": "Registrer",
        "unknown": "Ukendt",
        "download": "Download",
        "gridView": "Gittervisning",
        "listView": "Listevisning",
        "items": "Elementer"
    },
    "auth": {
        "login": "Log Ind",
        "register": "Registrer",
        "resetPassword": "Nulstil adgangskode",
        "setPassword": "Indstil adgangskode",
        "missingName": "Navn p\u00e5kr\u00e6vet",
        "missingEmail": "Email p\u00e5kr\u00e6vet",
        "missingPassword": "Adgangskode p\u00e5kr\u00e6vet",
        "repeatPassword": "Gentag adgangskode",
        "fullName": "Fulde navn",
        "email": "Email",
        "password": "Adgangskode",
        "newPassword": "Ny adgangskode",
        "emailNotVerified": "Email ikke verificeret. Tjek din indbakke. Tjek ogs\u00e5 spam.",
        "resendVerification": "Send verifikationsemail igen",
        "verificationResent": "Verifikationsemail sendt.",
        "invalidEmail": "Ugyldig email",
        "wrongCredentials": "Forkert email eller adgangskode",
        "passwordMismatch": "Adgangskoderne matcher ikke",
        "emailAlreadyInUse": "Email er allerede i brug",
        "userNotFound": "Ingen bruger med denne email",
        "logout": "Log ud",
        "verifyEmail": "Verificer din email",
        "verificationEmailSent": "Du burde have modtaget en verifikationsemail.",
        "useToActivate": "Brug den til at aktivere din konto.",
        "checkSpam": "Tjek om emailen er havnet i spam.",
        "checkEmail": "Tjek din email",
        "passwordResetSent": "Du burde have modtaget en email.",
        "useToResetPassword": "Brug den til at nulstille din adgangskode."
    },
    "content": {
        "addContent": "Tilf\u00f8j indhold",
        "confirmDelete": "Bekr\u00e6ft sletning",
        "confirmSubmit": "Bekr\u00e6ft indsendelse",
        "submitWarning": "N\u00e5r du har indsendt, er det ikke l\u00e6ngere muligt at redigere.",
        "submit": "Indsend",
        "authors": "Forfattere",
        "addAuthor": "Tilf\u00f8j forfatter",
        "addAtLeastOneAuthor": "Tilf\u00f8j mindst 1 forfatter",
        "imageAlt": "Indholdsbillede",
        "uploadFile": "Upload fil",
        "chooseFile": "Vælg en fil",
        "tableOfContents": "Indholdsfortegnelse"
    },
    "layout": {
        "welcomeTitle": "Velkommen til RadikalWiki",
        "loginOrRegister": "Log ind eller registrer dig.",
        "rememberEmail": "Husk at bruge den email du er registreret med i RU.",
        "greeting": "Hej {{name}}!",
        "acceptInvitations": "Accepter venligst dine invitationer til grupper og begivenheder.",
        "noInvitationsHint": "Hvis ingen invitationer dukker op, har du sandsynligvis brugt en anden email end den der er registreret hos Radikal Ungdom.",
        "groups": "Grupper",
        "events": "Begivenheder",
        "memberships": "Medlemskaber",
        "content": "Indhold",
        "noGroups": "Ingen grupper",
        "noEvents": "Ingen begivenheder",
        "noMemberships": "Ingen medlemskaber",
        "currentItem": "Nuv\u00e6rende punkt",
        "exitSearch": "Forlad s\u00f8gefeltet",
        "dark": "M\u00f8rk",
        "light": "Lys",
        "userMenu": "Brugermenu",
        "newestContent": "Nyeste indhold",
        "darkMode": "Mørk tilstand",
        "themeColor": "Temafarve",
        "primaryColor": "Primær",
        "accentColor": "Accent",
        "customColor": "Egen farve"
    },
    "error": {
        "somethingWentWrong": "Noget gik galt!",
        "sendMessage": "Send venligst f\u00f8lgende besked til"
    },
    "folder": {
        "manageFolder": "Administrer mappe",
        "proposedBy": "Foresl\u00e5et af",
        "export": "Eksporter",
        "copy": "Kopier",
        "paste": "Inds\u00e6t her",
        "lock": "L\u00e5s (bloker tilf\u00f8jelse)",
        "unlock": "L\u00e5s op (tillad tilf\u00f8jelse)"
    },
    "node": {
        "documentUnavailable": "Dokumentet er ikke tilg\u00e6ngeligt",
        "notFoundOrNoAccess": "Dette kan v\u00e6re fordi dokumentet ikke eksisterer, eller du ikke har adgang til det.",
        "maybeLoginForAccess": "Du kan muligvis f\u00e5 adgang til dokumentet ved at logge ind:"
    },
    "vote": {
        "newAmendment": "Nyt \u00c6ndringsforslag",
        "newComment": "Skriv en kommentar\u2026",
        "comments": "Kommentarer",
        "reply": "Svar",
        "replies": "svar",
        "question": "Sp\u00f8rgsm\u00e5l",
        "noAmendments": "Ingen \u00e6ndringsforslag",
        "noComments": "Ingen kommentarer endnu",
        "noQuestions": "Ingen sp\u00f8rgsm\u00e5l",
        "amendments": "\u00c6ndringsforslag",
        "questions": "Sp\u00f8rgsm\u00e5l",
        "candidates": "Kandidater",
        "hasVotingRight": "Du har stemmeret",
        "noVotingRight": "Du har ikke stemmeret",
        "hasNotVoted": "Du har ikke stemt",
        "hasVoted": "Du har stemt",
        "updateStatus": "Opdater status",
        "noVoteNow": "Ingen afstemning nu",
        "castVote": "Stem",
        "voteCount": "Antal stemmer"
    },
    "perm": {
        "type": "Type",
        "role": "Rolle",
        "insert": "Opret",
        "select": "Vis",
        "delete": "Slet",
        "active": "Aktiv"
    },
    "admin": {
        "poll": "Afstemning",
        "results": "Resultater",
        "votes": "Stemmer"
    },
    "speak": {
        "manageSpeakerList": "Administrer Talerlisten",
        "joinSpeakerList": "Kom p\u00e5 talerlisten",
        "open": "\u00c5ben",
        "close": "Luk",
        "talk": "Tal",
        "question": "Sp\u00f8rgsm\u00e5l",
        "clarify": "Opklar",
        "misunderstood": "Misforst\u00e5et",
        "procedure": "Procedure",
        "speakerList": "Talerliste",
        "removeFromList": "Fjern fra talerliste",
        "emptyList": "Talerlisten er tom",
        "clear": "Ryd",
        "start": "Start",
        "stop": "Stop",
        "speakingTime": "Taletid (s)",
        "speakingNow": "Taler nu",
        "yourTurn": "Din tur til at tale",
        "yourTurnBody": "Du er den nuværende taler.",
        "next": "Næste",
        "getReady": "Gør dig klar — du er den næste",
        "moveUp": "Flyt øverst",
        "moveDown": "Flyt nederst"
    },
    "poll": {
        "managePoll": "Administrer Afstemning",
        "hideResult": "Skjul resultatet",
        "newPoll": "Ny afstemning",
        "voteRange": "Antal stemmer",
        "start": "Start",
        "stopPoll": "Stop afstemning",
        "resultsHidden": "Resultat skjult"
    },
    "redirect": {
        "forwarding": "Sender dig videre til:",
        "noTarget": "Ingen omdirigering angivet endnu.",
        "targetUrl": "Mål-URL"
    },
    "social": {
        "empty": "Ingen opslag endnu.",
        "query": "Søgeord",
        "poweredBy": "Live fra Bluesky"
    },
    "profile": {
        "memberships": "Dine grupper og begivenheder",
        "signedInAs": "Logget ind som",
        "userId": "Bruger-ID"
    },
    "program": {
        "empty": "Ingen programpunkter endnu."
    },
    "parent": {
        "title": "Manglende forældre",
        "description": "Noder der har mistet deres forælder (forældreløse).",
        "none": "Ingen forældreløse noder."
    },
    "sort": {
        "saveSorting": "Gem sortering"
    },
    "invite": {
        "addAccess": "Tilf\u00f8j adgang",
        "noInvitations": "Ingen invitationer",
        "invitations": "Invitationer",
        "invite": "Inviter",
        "nameOrEmail": "Navn eller Email",
        "acceptInvitation": "Accepter invitation til {{name}}",
        "importRoster": "Importér liste (.xlsx: Fornavn, Efternavn, Email)",
        "imported": "Importerede {{count}} medlemmer",
        "noRosterRows": "Ingen rækker med en email fundet i filen"
    },
    "member": {
        "name": "Navn",
        "email": "EMail",
        "hidden": "Skjult",
        "hide": "Skjul medlem",
        "show": "Vis medlem",
        "owner": "Ejer",
        "active": "Aktiv",
        "actions": "Handlinger",
        "author": "Forfatter",
        "promote": "Gør til ejer",
        "demote": "Fjern ejer",
        "activate": "Markér aktiv",
        "deactivate": "Markér inaktiv",
        "remove": "Fjern medlem",
        "edit": "Rediger medlem",
        "confirmRemove": "Fjern dette medlem?",
        "export": "Eksportér deltagere (CSV)",
        "status": "Status",
        "filterAll": "Alle",
        "search": "Søg medlemmer"
    },
    "mime": {
        "group": "Gruppe",
        "event": "Begivenhed",
        "folder": "Mappe",
        "document": "Dokument",
        "file": "Fil",
        "person": "Person",
        "policy": "Politik",
        "position": "Position",
        "amendment": "\u00c6ndringsforslag",
        "candidate": "Kandidatur",
        "speakerList": "Talerliste",
        "editor": "Rediger",
        "sort": "Sorter",
        "speak": "Tal",
        "vote": "Afstemning",
        "members": "Medlemmer",
        "map": "Kort",
        "screen": "Skærm",
        "admin": "Resultater",
        "permissions": "Tilladelser",
        "graph": "Graf",
        "program": "Program",
        "profile": "Profil",
        "social": "Social væg",
        "parent": "Manglende forældre",
        "unknown": "Ukendt"
    }
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded JSON must parse and contain the sections the components use.
    /// A malformed blob would parse to an empty map and every key would silently
    /// fall back to itself (e.g. the literal "vote.noVoteNow" rendered on screen).
    #[test]
    fn translations_parse_and_cover_component_keys() {
        for (lang, map) in [("en", en_translations()), ("da", da_translations())] {
            assert!(!map.is_empty(), "{lang} translations failed to parse");
            for key in [
                "common.home",
                "vote.noVoteNow",
                "vote.castVote",
                "vote.amendments",
                "speak.emptyList",
                "speak.joinSpeakerList",
                "poll.managePoll",
                "sort.saveSorting",
                "invite.nameOrEmail",
                "member.author",
                "mime.vote",
            ] {
                assert!(
                    lookup_key(&map, key).is_some(),
                    "{lang} missing translation for {key}"
                );
            }
        }
    }
}
