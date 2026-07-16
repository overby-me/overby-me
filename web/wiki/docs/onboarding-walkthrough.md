# Onboarding-reality walkthrough: can a non-technical member obtain a DID and link it

Status: **PROPOSED protocol, not yet executed.** This document is the script for a timed walkthrough with 2 or 3 real assembly members. Running it requires owner sign-off, member consent, and the live interim app. Nothing here has been run against real users yet.

Round 2, item 16 of `docs/pre-rewrite-plan.md`. Related: the live DID audit result (`docs/pre-rewrite-plan.md`, "#3 audit result"), the PDS-agnostic identity decision (`docs/atproto-stack-decisions.md`), and the invite-to-bind flow (`docs/atproto-domain-model.md`).

## Why this exists

The identity migration assumes members self-register at any PDS (Bluesky, Eurosky, w.social, self-hosted) and link the resulting DID to their wiki profile via the profile card's "Link Bluesky account" flow (backend `/atproto/start` OAuth). The 2026-07-16 live audit found:

- 0 DIDs linked system-wide. The link flow has never been used by anyone.
- 20516 members, of which 83 percent are email-only roster rows that never logged in.

So the load-bearing assumption ("members can and will do this themselves") is completely untested against real users. This walkthrough tests it the only honest way: watch real members try, with a timer, and write down where they stall. A negative result forces an org-assisted DID provisioning path into the cutover plan now, instead of discovering the gap at migration time.

What this measures: whether the self-serve path is *achievable* for a non-technical member, and where it breaks. What it does not measure: unattended conversion at scale (the 4-to-6-week audit re-run below is for that), or whether members are *motivated* to do it without a facilitator watching.

## 1. Pilot selection

### Who

2 or 3 current members who already have (or can be given) a working wiki login. Span the tech-comfort range deliberately:

| Pilot | Tech comfort | Selection criterion |
|-|-|-|
| P1 | Low | Uses email and a browser; has never heard of Bluesky or "federated" anything. This is the member the migration must not lose. |
| P2 | Medium | Comfortable installing apps and creating accounts; not a developer. |
| P3 (optional) | High | Developer or power user. Acts as a control: if P3 stalls somewhere, that stall is a flow defect, not a skill gap. |

Do not recruit only volunteers who are excited about the technology; that self-selects away the population that matters. Ask the owner to nominate P1.

### PDS assignment

Two PDS choices, assigned (not chosen by the member), so both paths in the PDS-agnostic decision get exercised:

| Path | PDS | Assigned to | Why |
|-|-|-|-|
| Mainstream | `bsky.social` (signup at bsky.app) | P1, and P3 if present | The path most members will take at cutover must work for the *least* technical pilot. Assigning the alternative to P1 would confound "alternative PDS is rough" with "P1 is non-technical". |
| Alternative | One of Eurosky or w.social, whichever has open self-signup and working OAuth on the session date | P2 | Proves the PDS-agnostic claim with a non-Bluesky host. |

Facilitator pre-check, at most 24 hours before each session: personally complete a throwaway signup and a throwaway link on **both** assigned PDSes. If the alternative PDS has closed signups, a phone-verification wall, or a broken OAuth authorize page, substitute the other alternative and record the substitution in the measurement sheet. Do not show pilots any of this dry run.

### Setting

- The member's **own device** (their everyday phone or laptop), never a prepared lab machine.
- In person, or a video call with screen share. The facilitator must be able to see the screen at all times.
- One member per session. No group sessions (members would help each other and mask stalls).
- Consent up front: explain that the session is timed, that notes record what they click and say, and that results are shared internally. Handles and timings are recorded; no passwords, and no screen recording is kept unless the member explicitly agrees.

## 2. The script

Read each step to the member verbatim. Start the step timer when you finish reading; stop it when the "Done when" condition is visibly true. Do not paraphrase, demonstrate, or foreshadow the next step.

Step 0 is setup and is not timed.

| # | What the member is told (verbatim) | Done when | Target | Cap |
|-|-|-|-|-|
| 0 | "We're testing whether our instructions work, not testing you. If something is confusing, that's our bug. Please think out loud." Confirm device, screen visibility, consent. | Member agrees and screen is visible. | n/a | n/a |
| 1 | "You received this invite email from the assembly. Starting from that email, sign in to the wiki." (For a member who already uses the wiki: "Open the wiki and sign in as you normally would.") | The app home is on screen and the member is signed in. | 3 min | 8 min |
| 2 | "For the next part you need your own account on [bsky.app / the assigned PDS's signup page]. Create one. Tell me your new username when you have it." | Member states their new handle and is signed in at the PDS. | 7 min | 15 min |
| 3 | "Back in the wiki: open your own profile page." | The profile card (name, email) is on screen. | 1 min | 3 min |
| 4 | "Find the part of your profile about linking a Bluesky account, enter the username you just created, and continue." | The browser has navigated away to the PDS authorization page. | 2 min | 5 min |
| 5 | "Read the page you were sent to and respond to it however you think is right." | Back in the wiki with the "Bluesky account linked" confirmation visible. | 2 min | 5 min |
| 6 | "How would you check that it worked?" | Member locates "Linked as @handle" on the profile card (or an equivalent proof) unprompted. | 1 min | 3 min |

Notes on the phrasing:

- Step 2 deliberately does not walk through the PDS's own signup form; the PDS UX is part of what is being measured. If the PDS demands phone verification or an invite code, that is a finding, not a reason to help.
- Step 5 deliberately does not say "click Allow". Whether the member understands the OAuth consent screen well enough to approve it is exactly the question.
- Step 6 measures whether the success state is legible, not whether the member can be told where to look.

### Stall-point log

Keep one log row per observation, continuously, tied to the step in progress:

| Step | Clock | What they clicked / typed | Where they hesitated (verbatim if spoken) | What they asked |
|-|-|-|-|-|
| | | | | |

Log at minimum: every wrong click (what they clicked instead of the right thing), every pause longer than 30 seconds and what they were looking at during it, every question, and every utterance of confusion ("what's a handle?", "is this the same password?"). Verbatim quotes beat summaries.

## 3. Facilitator rules

1. **Do not help unless the member is stuck for more than 2 minutes.** "Stuck" means: no forward progress and either the member says they don't know what to do, or they have repeated the same failing action three times. Silent slow progress is not stuck; let it run to the step cap.
2. **Escalate interventions in order, and record every one verbatim** (what the member said, what you said, the clock time):
   - *Nudge*: re-read the step's script line, nothing more.
   - *Hint*: name the screen region ("look at the cards below your name"), never the exact control.
   - *Takeover*: perform the action for them. A takeover marks the step **failed** even though the session continues.
3. Never touch the member's device except during a takeover.
4. Answer questions only with "what do you think?" or a re-read of the step. Conceptual questions ("what is Bluesky?") get: "good question, I'm writing that down", and a log entry.
5. If a step hits its cap without completion, offer one hint; if that does not unstick within 2 more minutes, take over, mark the step failed, and continue (later steps still yield data).
6. Session hard cap: 45 minutes from step 1. If the cap hits, stop, mark the session incomplete, debrief kindly.
7. Immediately after each step, ask: "On a scale of 1 to 5, where 1 is very easy and 5 is very hard, how hard was that?" Record the number before reading the next step.
8. Debrief at the end: ask "if the email had just asked you to do this on your own, would you have?" Record the answer verbatim.
9. Reset between pilots: unlink any test linkage you created in the dry run; never reuse a handle across pilots.

## 4. Measurement sheet

One sheet per pilot. Fill during the session, not from memory afterwards.

Session header:

| Field | Value |
|-|-|
| Pilot (P1/P2/P3), tech comfort | |
| Date, facilitator | |
| Device + browser | |
| PDS assigned (and any substitution) | |
| App URL + frontend commit | |

Per step:

| Step | Minutes | Nudges | Hints | Takeovers | Completed (yes/no) | Difficulty (1-5) |
|-|-|-|-|-|-|-|
| 1 | | | | | | |
| 2 | | | | | | |
| 3 | | | | | | |
| 4 | | | | | | |
| 5 | | | | | | |
| 6 | | | | | | |

Session totals:

| Field | Value |
|-|-|
| Total minutes (step 1 start to step 6 done, or abort) | |
| Outcome (completed / completed-with-takeover / aborted) | |
| Total interventions | |
| "Would you have done this from the email alone?" (verbatim) | |
| One-line member quote that best captures the session | |

## 5. Interpretation

n = 2 or 3 is a smoke test, not a study. A pass is weak evidence (the path is not fundamentally broken); a fail is strong evidence (one observed failure among 2 or 3 facilitated best-case attempts predicts mass failure among 20516 unattended members). Read the thresholds with that asymmetry in mind.

### Triggers that force an org-assisted DID provisioning path into the cutover plan

Any one of these fires the trigger:

- **T1, hard fail:** any pilot fails any step outright (a takeover, or a step cap exceeded without completion). One facilitated failure in three attempts is a 33 percent floor on the unattended failure rate; at roster scale that is thousands of members.
- **T2, too slow:** median total time across pilots exceeds **20 minutes** (completed sessions only; an aborted session counts as T1 anyway). Rationale: the facilitated, scheduled, watched setting is the best case. Real members get an unattended email nudge; 20+ minutes of best-case friction on a volunteer task predicts drop-off, not completion.
- **T3, systemic stall:** two or more pilots need an intervention at the **same** stall point (same step, same confusion). A shared stall is a flow defect, not a skill gap, and it fires even if everyone eventually finished in time.
- **T4, mainstream signup wall:** step 2 on the mainstream path (bsky.social) fails or exceeds 10 minutes for any pilot. Account creation is the one step entirely outside our UI; if the mainstream PDS itself is the wall, no amount of wiki UX work fixes it, and provisioning is the only lever.

Soft signal, does not fire the trigger alone: mean difficulty of 4 or higher on any step, or a unanimous "no" to "would you have done this from the email alone?". Either one means the flow needs work before the invite-to-bind flow hardens, even if all timings passed.

"Org-assisted DID provisioning path" means the cutover plan must contain a designed alternative for members who cannot or will not self-register: for example the org batch-creating accounts on a PDS on members' behalf, an org-run PDS with pre-provisioned invite codes, or a guided in-app wizard that performs signup inline. Choosing among those is out of scope here; this walkthrough only decides whether the cutover plan must contain one.

### Proposed follow-up: re-run the DID audit in 4 to 6 weeks

Regardless of pilot outcome, PROPOSED (owner sign-off required, read-only): re-run the item 3 read-only DID audit 4 to 6 weeks after this walkthrough, while the link nudge (item 4) is live, and compare against the 2026-07-16 baseline of 0 linked DIDs. That re-run measures what the pilots cannot: unattended conversion.

Reading the re-run, against the 3507 active members who actually have accounts (the 83 percent roster-only cohort cannot link until they log in at all, so they are out of the denominator):

- Under 2 percent linked: self-serve conversion is effectively zero even among engaged members. Treat as T2-equivalent: the org-assisted path goes into the cutover plan.
- 2 to 10 percent linked: keep the nudge, but plan the assisted path for the long tail anyway.
- Over 10 percent and climbing: the self-serve assumption holds for the engaged cohort. The roster-only 83 percent still migrate as pending invites through the invite-to-bind flow regardless; this walkthrough never claimed to cover them.

Record the re-run numbers at the bottom of this document when they exist.
