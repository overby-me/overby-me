use std::collections::HashMap;
use std::sync::LazyLock;

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

/// Set the document's `<html lang>` so the browser can auto-hyphenate
/// (`hyphens: auto`) — without a `lang` attribute the browser has no dictionary
/// and hyphenation is a no-op. Mirrors the old wiki's `<html lang="da">`.
pub fn apply_lang(lang: &Lang) {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    {
        let _ = el.set_attribute("lang", lang.code());
    }
}

pub static LANG: GlobalSignal<Lang> = Signal::global(|| Lang::En);

/// Translation function — looks up key in nested translation map
pub fn t(key: &str) -> String {
    let lang = LANG.read();
    let translations = match *lang {
        Lang::En => en_translations(),
        Lang::Da => da_translations(),
    };

    lookup_key(translations, key).unwrap_or_else(|| key.to_string())
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

// The translation tables are parsed from the embedded JSON exactly once each and
// cached; `t()` runs on every render across 200+ call sites, so re-parsing ~29KB
// of JSON per call (the previous behaviour) dominated render CPU.
static EN_TABLE: LazyLock<HashMap<String, serde_json::Value>> =
    LazyLock::new(|| serde_json::from_str(EN_JSON).unwrap_or_default());
static DA_TABLE: LazyLock<HashMap<String, serde_json::Value>> =
    LazyLock::new(|| serde_json::from_str(DA_JSON).unwrap_or_default());

fn en_translations() -> &'static HashMap<String, serde_json::Value> {
    &EN_TABLE
}

fn da_translations() -> &'static HashMap<String, serde_json::Value> {
    &DA_TABLE
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
    "toc": {
        "title": "On this page",
        "empty": "No sections on this page"
    },
    "common": {
        "add": "Add",
        "cancel": "Cancel",
        "delete": "Delete",
        "edit": "Edit",
        "save": "Save",
        "previous": "Previous",
        "next": "Next",
        "tools": "Actions",
        "apps": "Apps",
        "backToTop": "Back to top",
        "copyLink": "Copy link",
        "linkCopied": "Link copied",
        "skipToContent": "Skip to content",
        "keyboardShortcuts": "Keyboard shortcuts",
        "shortcutHelp": "Show this help",
        "noResults": "No results",
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
        "noPreview": "No preview available for this file",
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
        "shareBluesky": "Share to Bluesky",
        "sharing": "Sharing to Bluesky…",
        "shared": "Shared to Bluesky",
        "shareNoLink": "Link your Bluesky account first (in your profile)",
        "shareErr": "Could not share to Bluesky",
        "projectScreen": "Project on screen",
        "projected": "Now on the screen",
        "showCommentsScreen": "Show comments on screen",
        "hideCommentsScreen": "Hide comments on screen",
        "commentsOnScreen": "Comments now shown on the screen",
        "commentsOffScreen": "Comments hidden from the screen",
        "confirmDelete": "Confirm Deletion",
        "confirmSubmit": "Confirm Submission",
        "submitWarning": "Once you have submitted, it is no longer possible to edit.",
        "submit": "Submit",
        "authors": "Authors",
        "addAuthor": "Add Author",
        "addAtLeastOneAuthor": "Add at least 1 author",
        "uploadFile": "Upload File",
        "chooseFile": "Choose a file",
        "imageAlt": "Content image",
        "coverImage": "Cover image",
        "uploadImage": "Upload a cover image",
        "removeImage": "Remove image",
        "date": "Date"
    },
    "layout": {
        "welcomeTitle": "Welcome to RadikalWiki",
        "greetMorning": "Good morning",
        "greetAfternoon": "Good afternoon",
        "greetEvening": "Good evening",
        "greetNight": "Still up?",
        "loginOrRegister": "Log in or register.",
        "rememberEmail": "Remember to use the email you registered with at RU.",
        "greeting": "Hello {{name}}!",
        "acceptInvitations": "Please accept your invitations to groups and events.",
        "noInvitationsHint": "If no invitations appear, you most likely used a different email than the one registered with Radikal Ungdom.",
        "newGroup": "New group",
        "newEvent": "New event",
        "createFailed": "Could not create it. Try a different name.",
        "showAll": "Show all ({{count}})",
        "showMore": "Show more",
        "showLess": "Show less",
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
        "account": "Account",
        "profile": "Profile",
        "newestContent": "Newest content",
        "darkMode": "Dark mode",
        "compactDensity": "Compact density",
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
        "exporting": "Generating export…",
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
        "sending": "Sending…",
        "comments": "Comments",
        "reply": "Reply",
        "replies": "replies",
        "replyNotifyTitle": "New reply",
        "replyNotifyBody": "Someone replied to your comment",
        "question": "Question",
        "noComments": "No comments yet",
        "noQuestions": "No questions",
        "noAmendments": "No amendments yet",
        "confirmDeleteComment": "Delete this comment and its replies?",
        "blankOnlyAlone": "Blank cannot be combined with other choices",
        "selectAtLeast": "Select at least {{count}}",
        "selectAtMost": "Select at most {{count}}",
        "amendments": "Amendments",
        "questions": "Questions",
        "candidates": "Candidates",
        "noCandidates": "No candidates yet",
        "addCandidate": "Add candidate",
        "candidatePhoto": "Photo (optional)",
        "photoReady": "Photo ready",
        "hasVotingRight": "You have voting rights",
        "noVotingRight": "You do not have voting rights",
        "hasNotVoted": "You have not voted",
        "hasVoted": "You have voted",
        "updateStatus": "Update status",
        "noVoteNow": "No vote right now",
        "castVote": "Vote",
        "voteCount": "Number of votes",
        "resultWinner": "Result",
        "resultTie": "Tie — not carried",
        "resultNone": "No result",
        "turnout": "Turnout",
        "eligible": "eligible",
        "closed": "Voting closed",
        "pollOpenTitle": "Vote open",
        "pollOpenBody": "A vote has opened — cast your ballot"
    },
    "perm": {
        "type": "Type",
        "role": "Role",
        "insert": "Create",
        "select": "View",
        "delete": "Delete",
        "active": "Active",
        "granted": "granted",
        "denied": "denied"
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
        "noLists": "No speaker lists yet",
        "newList": "New speaker list",
        "clear": "Clear",
        "confirmClear": "Clear the whole speaker list?",
        "start": "Start",
        "stop": "Stop",
        "speakingTime": "Speaking time (s)",
        "nextSpeaker": "Next speaker",
        "remaining": "Remaining speaking time",
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
        "hideResult": "Hide running tally (not a secret ballot)",
        "secretBallot": "Secret ballot (anonymous)",
        "newPoll": "New poll",
        "voteRange": "Number of votes",
        "minVotesLabel": "Minimum number of votes",
        "maxVotesLabel": "Maximum number of votes",
        "start": "Start",
        "stopPoll": "Stop poll",
        "resultsHidden": "Running tally hidden until the poll closes"
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
        "sharedMemberships": "Groups and events you share",
        "contributions": "Your contributions",
        "signedInAs": "Signed in as",
        "viewProfile": "View profile",
        "userId": "User ID",
        "linkBluesky": "Link Bluesky account",
        "linkBlueskyHint": "Connect your Bluesky (atproto) account to this profile.",
        "blueskyHandle": "Bluesky handle",
        "linkedOk": "Bluesky account linked",
        "linkedErr": "Could not link Bluesky account",
        "blueskyAccount": "Bluesky account",
        "linkedAs": "Linked as",
        "unlink": "Unlink",
        "unlinkedOk": "Bluesky account unlinked",
        "unlinkErr": "Could not unlink Bluesky account",
        "notifications": "Notifications",
        "notifyHint": "Get notified on this device when a vote opens in a group you're in, even when the app is closed.",
        "enableNotify": "Enable notifications",
        "disableNotify": "Disable notifications",
        "notifyEnabled": "Notifications enabled on this device",
        "notifyDisabled": "Notifications disabled",
        "notifyErr": "Could not enable notifications"
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
        "sent": "Invitation sent",
        "nameOrEmail": "Name or Email",
        "acceptInvitation": "Accept invitation to {{name}}",
        "invited": "Invited",
        "declineInvitation": "Decline invitation",
        "confirmReject": "Reject this invitation?",
        "reject": "Reject",
        "importRoster": "Import roster (.xlsx: Fornavn, Efternavn, Email)",
        "imported": "Imported {{count}} members",
        "noRosterRows": "No rows with an email found in the file",
        "copyClaimLink": "Copy claim link",
        "claimLinkCopied": "Claim link copied — send it to this person",
        "claimedOk": "Invitation claimed",
        "claimErr": "Could not claim this invitation"
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
    "follow": {
        "live": "Following the room, live",
        "idle": "Nothing is being presented right now."
    },
    "console": {
        "title": "Console",
        "agenda": "Agenda",
        "onScreen": "On screen",
        "stopProjecting": "Stop projecting",
        "focusSection": "Focus a section on screen",
        "focusWhole": "Show the whole document",
        "focusNoSections": "This item has no sections to focus."
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
        "follow": "Follow",
        "admin": "Results",
        "permissions": "Permissions",
        "graph": "Graph",
        "program": "Programme",
        "profile": "Profile",
        "social": "Social wall",
        "parent": "Missing parents",
        "redirect": "Redirect",
        "cow": "Cow",
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
    "toc": {
        "title": "P\u00e5 denne side",
        "empty": "Ingen sektioner p\u00e5 denne side"
    },
    "common": {
        "add": "Tilf\u00f8j",
        "cancel": "Annuller",
        "delete": "Slet",
        "edit": "Rediger",
        "save": "Gem",
        "previous": "Forrige",
        "next": "N\u00e6ste",
        "tools": "Handlinger",
        "apps": "Apps",
        "backToTop": "Tilbage til toppen",
        "copyLink": "Kopi\u00e9r link",
        "linkCopied": "Link kopieret",
        "skipToContent": "G\u00e5 til indhold",
        "keyboardShortcuts": "Tastaturgenveje",
        "shortcutHelp": "Vis denne hj\u00e6lp",
        "noResults": "Ingen resultater",
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
        "noPreview": "Ingen forhåndsvisning for denne fil",
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
        "shareBluesky": "Del p\u00e5 Bluesky",
        "sharing": "Deler p\u00e5 Bluesky\u2026",
        "shared": "Delt p\u00e5 Bluesky",
        "shareNoLink": "Forbind din Bluesky-konto f\u00f8rst (p\u00e5 din profil)",
        "shareErr": "Kunne ikke dele p\u00e5 Bluesky",
        "projectScreen": "Vis p\u00e5 sk\u00e6rmen",
        "projected": "Nu p\u00e5 sk\u00e6rmen",
        "showCommentsScreen": "Vis kommentarer p\u00e5 sk\u00e6rmen",
        "hideCommentsScreen": "Skjul kommentarer p\u00e5 sk\u00e6rmen",
        "commentsOnScreen": "Kommentarer vises nu p\u00e5 sk\u00e6rmen",
        "commentsOffScreen": "Kommentarer skjult fra sk\u00e6rmen",
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
        "coverImage": "Forsidebillede",
        "uploadImage": "Upload et forsidebillede",
        "removeImage": "Fjern billede",
        "date": "Dato"
    },
    "layout": {
        "welcomeTitle": "Velkommen til RadikalWiki",
        "greetMorning": "Godmorgen",
        "greetAfternoon": "God eftermiddag",
        "greetEvening": "Godaften",
        "greetNight": "Stadig oppe?",
        "loginOrRegister": "Log ind eller registrer dig.",
        "rememberEmail": "Husk at bruge den email du er registreret med i RU.",
        "greeting": "Hej {{name}}!",
        "acceptInvitations": "Accepter venligst dine invitationer til grupper og begivenheder.",
        "noInvitationsHint": "Hvis ingen invitationer dukker op, har du sandsynligvis brugt en anden email end den der er registreret hos Radikal Ungdom.",
        "newGroup": "Ny gruppe",
        "newEvent": "Ny begivenhed",
        "createFailed": "Kunne ikke oprette. Prøv et andet navn.",
        "showAll": "Vis alle ({{count}})",
        "showMore": "Vis mere",
        "showLess": "Vis mindre",
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
        "account": "Konto",
        "profile": "Profil",
        "notSubmitted": "Ikke indsendt",
        "newestContent": "Nyeste indhold",
        "darkMode": "Mørk tilstand",
        "compactDensity": "Kompakt visning",
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
        "exporting": "Genererer eksport\u2026",
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
        "sending": "Sender\u2026",
        "comments": "Kommentarer",
        "reply": "Svar",
        "replies": "svar",
        "replyNotifyTitle": "Nyt svar",
        "replyNotifyBody": "Nogen svarede p\u00e5 din kommentar",
        "question": "Sp\u00f8rgsm\u00e5l",
        "noAmendments": "Ingen \u00e6ndringsforslag",
        "noComments": "Ingen kommentarer endnu",
        "noQuestions": "Ingen sp\u00f8rgsm\u00e5l",
        "confirmDeleteComment": "Slet denne kommentar og dens svar?",
        "blankOnlyAlone": "Blank kan ikke kombineres med andre valg",
        "selectAtLeast": "V\u00e6lg mindst {{count}}",
        "selectAtMost": "V\u00e6lg h\u00f8jst {{count}}",
        "amendments": "\u00c6ndringsforslag",
        "questions": "Sp\u00f8rgsm\u00e5l",
        "candidates": "Kandidater",
        "noCandidates": "Ingen kandidater endnu",
        "addCandidate": "Tilf\u00f8j kandidat",
        "candidatePhoto": "Foto (valgfrit)",
        "photoReady": "Foto klar",
        "hasVotingRight": "Du har stemmeret",
        "noVotingRight": "Du har ikke stemmeret",
        "hasNotVoted": "Du har ikke stemt",
        "hasVoted": "Du har stemt",
        "updateStatus": "Opdater status",
        "noVoteNow": "Ingen afstemning nu",
        "castVote": "Stem",
        "voteCount": "Antal stemmer",
        "resultWinner": "Resultat",
        "resultTie": "Uafgjort — ikke vedtaget",
        "resultNone": "Intet resultat",
        "turnout": "Stemmeprocent",
        "eligible": "stemmeberettigede",
        "closed": "Afstemning afsluttet",
        "pollOpenTitle": "Afstemning åben",
        "pollOpenBody": "En afstemning er åbnet — afgiv din stemme"
    },
    "perm": {
        "type": "Type",
        "role": "Rolle",
        "insert": "Opret",
        "select": "Vis",
        "delete": "Slet",
        "active": "Aktiv",
        "granted": "tildelt",
        "denied": "ikke tildelt"
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
        "noLists": "Ingen talerlister endnu",
        "newList": "Ny talerliste",
        "clear": "Ryd",
        "confirmClear": "Ryd hele talerlisten?",
        "start": "Start",
        "stop": "Stop",
        "speakingTime": "Taletid (s)",
        "nextSpeaker": "Næste taler",
        "remaining": "Resterende taletid",
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
        "hideResult": "Skjul løbende optælling (ikke hemmelig afstemning)",
        "secretBallot": "Hemmelig afstemning (anonym)",
        "newPoll": "Ny afstemning",
        "voteRange": "Antal stemmer",
        "minVotesLabel": "Mindste antal stemmer",
        "maxVotesLabel": "Største antal stemmer",
        "start": "Start",
        "stopPoll": "Stop afstemning",
        "resultsHidden": "Løbende optælling skjult indtil afstemningen lukkes"
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
        "sharedMemberships": "Grupper og begivenheder I deler",
        "contributions": "Dine bidrag",
        "signedInAs": "Logget ind som",
        "viewProfile": "Vis profil",
        "userId": "Bruger-ID",
        "linkBluesky": "Forbind Bluesky-konto",
        "linkBlueskyHint": "Forbind din Bluesky (atproto) konto til denne profil.",
        "blueskyHandle": "Bluesky-håndtag",
        "linkedOk": "Bluesky-konto forbundet",
        "linkedErr": "Kunne ikke forbinde Bluesky-konto",
        "blueskyAccount": "Bluesky-konto",
        "linkedAs": "Forbundet som",
        "unlink": "Fjern forbindelse",
        "unlinkedOk": "Bluesky-forbindelse fjernet",
        "unlinkErr": "Kunne ikke fjerne Bluesky-forbindelse",
        "notifications": "Notifikationer",
        "notifyHint": "Bliv notificeret på denne enhed, når en afstemning åbner i en gruppe, du er med i, også når appen er lukket.",
        "enableNotify": "Slå notifikationer til",
        "disableNotify": "Slå notifikationer fra",
        "notifyEnabled": "Notifikationer slået til på denne enhed",
        "notifyDisabled": "Notifikationer slået fra",
        "notifyErr": "Kunne ikke slå notifikationer til"
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
        "sent": "Invitation sendt",
        "nameOrEmail": "Navn eller Email",
        "acceptInvitation": "Accepter invitation til {{name}}",
        "invited": "Inviteret",
        "declineInvitation": "Afvis invitation",
        "confirmReject": "Afvis denne invitation?",
        "reject": "Afvis",
        "importRoster": "Importér liste (.xlsx: Fornavn, Efternavn, Email)",
        "imported": "Importerede {{count}} medlemmer",
        "noRosterRows": "Ingen rækker med en email fundet i filen",
        "copyClaimLink": "Kopiér tilmeldingslink",
        "claimLinkCopied": "Tilmeldingslink kopieret — send det til personen",
        "claimedOk": "Invitation tilmeldt",
        "claimErr": "Kunne ikke tilmelde denne invitation"
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
    "follow": {
        "live": "Følger salen, live",
        "idle": "Der vises ikke noget lige nu."
    },
    "console": {
        "title": "Konsol",
        "agenda": "Dagsorden",
        "onScreen": "På skærmen",
        "stopProjecting": "Stop projektion",
        "focusSection": "Fokusér en sektion på skærmen",
        "focusWhole": "Vis hele dokumentet",
        "focusNoSections": "Dette element har ingen sektioner at fokusere på."
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
        "follow": "Følg",
        "admin": "Resultater",
        "permissions": "Tilladelser",
        "graph": "Graf",
        "program": "Program",
        "profile": "Profil",
        "social": "Social væg",
        "parent": "Manglende forældre",
        "redirect": "Omdirigering",
        "cow": "Ko",
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

    /// English and Danish must expose exactly the same set of leaf keys, so no
    /// locale renders a raw `section.key` fallback to users (t() falls back to the
    /// key string). Guards against drift when keys are added to one map only.
    #[test]
    fn en_da_key_sets_match() {
        fn leaf_keys(
            map: &HashMap<String, serde_json::Value>,
        ) -> std::collections::BTreeSet<String> {
            let mut out = std::collections::BTreeSet::new();
            for (section, value) in map {
                if let Some(obj) = value.as_object() {
                    for k in obj.keys() {
                        out.insert(format!("{section}.{k}"));
                    }
                } else {
                    out.insert(section.clone());
                }
            }
            out
        }
        let en = leaf_keys(&en_translations());
        let da = leaf_keys(&da_translations());
        let en_only: Vec<_> = en.difference(&da).collect();
        let da_only: Vec<_> = da.difference(&en).collect();
        assert!(
            en_only.is_empty() && da_only.is_empty(),
            "i18n key drift — en-only: {en_only:?}, da-only: {da_only:?}"
        );
    }
}
