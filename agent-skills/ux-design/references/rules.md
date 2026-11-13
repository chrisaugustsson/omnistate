# UX rules

The rules every design follows and every judge or reviewer checks. Each rule states what a design must do; **Check:** adds what the judge looks for where that is not obvious from the rule; exceptions are in brackets. Findings cite rules by ID (S6, F2, ...). Sources are in [sources.md](sources.md).

## Common AI mistakes

Check these first. They are what generated UIs get wrong most often.

- **Happy path only:** tidy sample data; no empty, loading, error, partial, selection, overflow or offline states; "no data" shown while loading. (S1, S2, S15)
- **Emphasis in the wrong place:** a hero tile with one number, secondary goals on top, no focal point, the same block twice, pages where each part is fine but the whole has no hierarchy. (M3, H1)
- **A primary button on every card**, labels like "Submit", "OK", "Learn more", "Get started". (H1, H2)
- **Modals for everything:** create, edit and view in modals, modal on modal, no deep links, Back loses filters. (K1, N6, N7)
- **System words in the UI:** navigation named after the data model ("Entities", "Records", "Configs") or vague verbs ("Explore", "Insights"). (M2)
- **Landing-page forms:** placeholder as label, two columns, bare asterisks. (F1, F2, F3)
- **Errors in toasts** with generic text, input lost, "Are you sure?" with Yes/No on every delete. (S6, S7, S8, S9)
- **Decorative dashboards:** KPI cards with no target or change, not clickable, donut charts. (P7)
- **Phone as squeezed desktop:** a shrunken table, a hamburger, actions in a top corner, hover menus kept. (R1, R2, R4)
- **Hover-only and drag-only actions**, clickable rows with no real control, headings as styled text, unlabelled icon buttons. (P5, P11, A1, A4)
- **A welcome tour** instead of teaching in the empty state. (P9)
- **Frequent tasks wrapped in wizards**; search and filters on tiny lists but none on big ones. (N8, P3, P10)


## Model and language (M)

- **M1** Every object and action has one name, one home view and one set of verbs, the same on every screen. **Check:** each spec noun maps to one label and one view; no synonyms for one thing; a function has one label everywhere ("Save", never also "Apply" or "Done").
- **M2** Labels use the users' words and content follows the real task's order. **Check:** no entity or table names, enum values, camelCase or codes; no vague labels; invented terms explained. [Developer tools whose users speak that vocabulary.]
- **M3** Each screen's title and first line say what it is for, and every element traces to a Flows item and appears once. **Check:** orphan elements, duplicated blocks, large tiles holding one low-value number. [A summary repeated beside a confirm button.]
- **M4** Plain language: common words, active voice, sentences of 25 words or fewer, jargon defined on first use, real copy. **Check:** read every string. [Terms the audience really uses.]
- **M5** The system carries complexity the user should not: it asks only for what it cannot know or compute, and accepts input as people naturally type it. **Check:** no fields for data already held; totals computed; one free-form field with an example where formats vary. [Deliberate re-entry for safety.]

## Hierarchy and layout (H)

- **H1** At most one primary action per view, dialog, form step or repeated unit, serving the main job. **Check:** count filled buttons per region. [Choices between equals, such as plan pickers.]
- **H2** Buttons are a verb naming the result ("Send invite", "Delete project"); the last wizard step names the final action. **Check:** every button starts with a verb. [Pure acknowledgements; "Continue" on question pages.]
- **H3** Related things sit closer than unrelated ones; every group has a heading or shared container; forms over 6 fields have titled sections. **Check:** gaps between groups clearly larger than within; labels nearest their own field; actions beside the group they act on.
- **H4** What behaves the same looks the same; every interactive element has a visible signifier and nothing static imitates one. **Check:** one button shape; headings not clickable-looking; icons carry text unless universal (search, close, menu); destructive actions styled apart.
- **H5** Controls sit next to the object they affect. **Check:** row actions in the row; bulk actions in a bar above the list; global actions in the app bar.
- **H6** Frequent targets are large and near the user's attention; destructive targets sit away from frequent ones. **Check:** primary not smaller than secondary; Delete not beside Save; submit at the end of what it submits.
- **H7** Common things follow convention: logo top left linking home, search at the top, account top right, standard controls for their standard purpose. **Check:** custom widgets where a standard one exists. [New interactions: stronger signifiers, a fallback, *needs user testing*.]
- **H8** Few choices per decision; rare and advanced options behind one labelled disclosure, at most two levels deep; everything frequent tasks need stays visible. **Check:** the default view covers every frequent job; the disclosure says what it reveals. A menu does not fail only for having more than 7 items.
- **H9** The most important items go first or last in navigation and toolbars. [Inherently ordered sequences.]

## Navigation and wayfinding (N)

- **N1** Global navigation is identical on every screen and marks the current place by more than colour. **Check:** same items in the same order; one active item; logo links home. [Focused flows may hide it but keep an exit.]
- **N2** Top-level navigation is visible: a sidebar or top bar on desktop; at most 5 items in a bottom bar on phone; any collapsed menu has a text label.
- **N3** Local navigation for sections with sibling pages, weaker than the global nav; breadcrumbs only for hierarchies of 3+ levels, showing hierarchy, ending in the unlinked current page.
- **N4** Every screen has a unique title that echoes the link that led there; detail screens carry the object's name.
- **N5** Every screen shows where the user is, what is selected, and which mode, scope or filter is active. **Check:** active nav item, selected rows marked, active filters as removable chips, edit or view mode labelled.
- **N6** Meaningful state (selected record, tab, filters, query) is addressable and survives reload, Back and sharing. **Check:** each screen annotated with a route; Back from detail returns to the list with filters and scroll kept. [Transient dialogs.]
- **N7** Every non-root view and every flow step has a visible way out, and Back returns to the previous view as it was left. **Check:** an exit on every modal, step and full-screen sheet; wizard Back keeps entered data.
- **N8** Frequent tasks start one action from where users work, without wizard, modal chain or detour. Count interaction cost, not clicks. **Check:** for each frequent job, its start control is visible on its entry screen; no step only confirms a non-destructive action.
- **N9** What a decision needs is visible where it is made; the user never carries a value between screens. **Check:** confirm steps repeat what is confirmed; earlier answers summarised on later steps; choices picked, not typed from memory.

## Containers (K)

- **K1** The container fits the task:
  - **Inline:** one independent value changed in place (rename, toggle, table cell).
  - **Modal:** a short answer needed before continuing (confirm delete, name a new folder); about 5 fields at most; never stacked; never for content compared with the page behind; never for promotion. One that would scroll becomes a page.
  - **Side panel (non-modal):** secondary work where the page behind stays relevant: a record opened from a list, filters.
  - **Page:** many or interdependent fields, multi-step flows, anything with its own URL, creating the product's main objects.
- **K2** Tabs for 2 to 6 sibling views of one object that need no side-by-side comparison, the default tab holding what matters most; never for sequential steps. A sidebar for app navigation with more than about 5 destinations or 2 levels on desktop. Separate pages for independent tasks and anything deep-linked.

## Forms and input (F)

- **F1** One column, labels above fields. [Short related pairs such as city and postcode.]
- **F2** Every input has a visible label outside the field; a placeholder only adds an example; format, length and unit hints sit under the label. **Check:** label and hint per field; icon-only buttons have a named label. [One search box with a search button.]
- **F3** Ask only for what the spec needs. Mark optional fields "(optional)", leave required ones unmarked, keep optional fields rare. **Check:** every field maps to a Flows item; no bare asterisks; more than 2 optional fields flagged.
- **F4** The control matches the data: radios for 2 to 5 options; typed input for numbers, dates of birth and long known lists (with autocomplete); pickers or steppers for bounded values; field width matching the expected length. **Check:** no dropdown with under 5 options or for a number.
- **F5** Prefill known and likely values, never arbitrary ones; never ask again for what the flow already has. **Check:** known data prefilled; "same as" options; choice groups not defaulting to the first item. [Consent and sensitive choices start empty; password confirmation.]
- **F6** Long or branching flows for infrequent users: one thing per page, a step indicator, Back that keeps data, Save and exit, and a review page whose Change links return to it. [Frequent or expert tasks stay on one page; 1 or 2 steps need no review.]
- **F7** Inline edit for one independent property; a panel or page with one Save for several related properties (more than about 4).
- **F8** Settings: grouped in sections, one save model per group. Toggles save at once with a "Saved" mark; forms save with one labelled button. Options changed often also live in the main UI.

## States and feedback (S)

- **S1** Every screen or region showing data has its state set drawn: ideal, loading, empty, error, partial, and success where the user acts. **Check:** a labelled frame or annotation per state; a missing one fails unless marked "N/A because". [Static pages.]
- **S2** Empty states say why and offer one way forward; no-results keeps the query and offers to clear filters; loading and empty are separate frames. **Check:** title, one line, one action; loading never shows "no items".
- **S3** Every action is acknowledged at once: a busy state, then a result state; double submission blocked. **Check:** save buttons have a "Saving…" variant; every button leads to a visible next state.
- **S4** Loading indicator by expected duration: nothing under about 1 s; a skeleton (whole page) or spinner (one module) for 2 to 10 s; above 10 s a progress bar with an estimate and Cancel. Skeletons mirror the real layout. **Check:** loading frames annotated with duration.
- **S5** Optimistic updates have a drawn rollback: the item reverts and an error with Retry appears beside it. [Payments, irreversible and server-validated actions use a pending state instead.]
- **S6** Reversible destructive actions run at once and offer Undo; confirmation dialogs are kept for irreversible, costly, bulk or other-people-affecting actions. A confirmation names the object and the consequence, its buttons are verbs ("Delete report" / "Keep report"), and the destructive one is not the default. **Check:** each destructive action annotated undo or confirm, with a reason.
- **S7** Error messages say what happened, why if known, and how to fix it, with a recovery control, in plain words. **Check:** every error string holds a cause or an instruction.
- **S8** Field errors sit at their field; several errors also get a summary at the top linking to each; entered values stay in place; validation runs on leaving the field or on submit, and the error clears on the fixing keystroke. [Password and card code fields may clear, saying so.]
- **S9** The channel matches scope and urgency: inline for one element, a banner for page or system conditions, a modal only for critical blockers, a toast only for low-stakes confirmations. Toasts hold at most one action (Undo), stay while they have one, and carry neither errors nor the only path to anything.
- **S10** Success shows where the action happened: the new item highlighted in its list, "Saved" beside the form. Multi-step flows end on a screen with the outcome and the next action.
- **S11** Unsaved work is guarded: autosave with a visible status (Saving…, Saved, Couldn't save – Retry), or a changed marker and a specific "Discard changes?" on leaving. Error, timeout and expiry screens say whether input was kept and how to resume. [Single inline edits committing on Enter.]
- **S12** Offline is designed: an indicator, cached content usable, edits queued as pending, sync confirmed. [Server-only actions disabled with a reason.]
- **S13** No permission is a designed state: what is restricted, why, how to request access; disabled controls state why. [Hide a feature entirely when knowing it exists would leak information.]
- **S14** Concurrent edits are shown, never silently overwritten: who changed what, and review, overwrite or discard. Changes by others or the system are explained. [Live co-editing uses presence instead, P8.]
- **S15** Layouts hold extreme content: longest names, large counts, 1000+ rows, missing optional fields; truncated text can be shown in full; long lists paginate or load more. **Check:** at least one frame with extreme data.

## Complex apps and power users (P)

- **P1** Every shortcut, gesture or palette command has a visible equivalent; shortcuts are printed beside their command in menus; a command palette has a visible trigger with its key hint. [No shortcuts on phone; apps with under about 10 commands need no palette.]
- **P2** Frequent jobs get an accelerator (bulk action, shortcut, saved view, preset) layered on the novice path. **Check:** an annotated accelerator per frequent job.
- **P3** Tables: first column a readable name; related columns together; sortable headers showing the current direction; filters as removable chips with Clear all; a visible search box on collections over about 20 items.
- **P4** Selection: row checkboxes, select-all with an in-between state, and once anything is selected a bulk bar with "N selected", the actions and Cancel; destructive bulk actions confirmed.
- **P5** Row actions: one or two visible, the rest in a persistent "⋯" menu, all reachable without hover.
- **P6** List rows show what a user needs to decide without opening (the spec's comparison fields), in the same place on every row; the selected row is marked in list-detail. Long tables keep a sticky header (and first column when scrolling sideways) and offer density and column settings. Repeated filtered lists can be saved as named views.
- **P7** Dashboard tiles show a value with its comparison (target, previous period, threshold), link to the list behind them, and each changes a decision; only exceptions are highlighted. [Bar and line charts over pie, gauge and 3D.]
- **P8** Shared objects show who is here, where they are working, and a history (who, what, which object, when) with restore. Comments anchor to what they discuss, sit in a side pane (a sheet on phone) and can be resolved.
- **P9** Teach in context: first-use empty states and hints when a feature first matters. Any tour is skippable and can be reopened. Long tasks start by saying what the user will need (documents, time, fees).
- **P10** Linear wizards only for rare, novice tasks; repeated work is hub-and-spoke or list-detail where any part opens directly.
- **P11** Every drag (reorder, move between columns, resize, canvas move, file drop) has a single-pointer path: "Move to…", up and down, numeric position fields, "Choose file". [Freehand drawing.]

## Responsive (R)

- **R1** Layout changes by width: phone (under 600) one pane and a bottom bar; tablet (600 to 839) a rail and one or two panes; desktop (840 and up) a sidebar or rail with list and detail side by side. On phone, opening an item replaces the list with the detail and a Back control.
- **R2** On phone, the primary action and top-level navigation sit at the bottom or mid-screen; destructive actions sit away from the primary.
- **R3** Bottom sheets only for short contextual choices: a visible close, Back works, one at a time. Core flows are full screens.
- **R4** Wide tables on phone become cards (name plus 2 or 3 key values) or a sideways scroll with a frozen first column and visible overflow; filters and sort behind one Filter button. [Tables of 3 columns or fewer.]
- **R5** Phone keeps the jobs the role does on the go (check, approve, comment, quick edit), and every cut is stated in an annotation. Touch screens use tap paths for everything hover offers on desktop.

## Accessibility at wireframe stage (A)

- **A1** Every action is reachable by keyboard through a real control (button, link, field). **Check:** each Flows action maps to a focusable control; row clicks, hover and right-click are extra paths only. [Freehand input.]
- **A2** Focus order follows reading order; dialogs take focus on open and return it to their trigger on close; sticky bars, banners and toasts leave focused elements visible. **Check:** a numbered focus order on each main screen.
- **A3** Targets at least 24 × 24 px with spacing; 44 × 44 for primary and all touch controls (iOS 44 pt, Android 48 dp). [Inline links in text.]
- **A4** One H1 per screen stating its purpose, headings nested in order, labelled regions (header, nav, main, aside), a skip link where navigation repeats.
- **A5** Meaning rests on text or icon plus text, never on colour, position or hover alone; hover content is also reachable by focus and tap. Grayscale wireframes are the test.
- **A6** Help that appears on several screens sits in the same place on each.
- **A7** Sign-in allows paste and password managers, offers a path without memory (passkey, magic link, SSO) and uses one code field.
- **A8** Timeouts warn at least 20 s ahead with a one-action extend, and input survives them; moving content can be paused. [Real-time events.]
- **A9** Context and layout change only on a user action; every animation has a reduced-motion alternative.
- **A10** Mid-task interruptions are limited to critical confirmations.

## When rules conflict

- **Consistency or the task:** consistency by default; deviate only for a clear task gain, and make the deviation look different.
- **Convention or innovation:** convention for common patterns; innovation gets stronger signifiers and *needs user testing*.
- **Minimal or discoverable:** decide by frequency. Frequent and essential stays visible; only rare and advanced is disclosed.
- **Prevent errors or stay fast:** prefer undo; confirm only what is irreversible and costly.
- **Novice or expert:** both, with accelerators layered on a clear default path.
- **Constrain or accept:** pickers for bounded sets, flexible parsing for free text.
- **Emphasis:** works only while rare, so one emphasised thing per region.


## Left for the UI stage

Contrast of text, non-text elements and focus indicators (WCAG 1.4.3, 1.4.11, 2.4.13); focus ring styling; type size, line height and text spacing (1.4.12); reflow and 400% zoom (1.4.10); final alt text; ARIA in code; motion timing and easing.
