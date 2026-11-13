export const meta = {
  name: 'design-feature',
  description: 'Second half of designing a feature: draws the layout the human picked as the finished design in OmniState, one board per screen or state on the project\'s design system, wired into a prototype. Run again later, it changes only the boards with open comments and the boards of situations that changed in the behaviour tree.',
  whenToUse: 'After design-options, when the human has picked a layout option (or combined them), and again whenever the human has commented on the finished boards or changed the behaviour tree. Run with the Workflow tool as the workflow design-feature with args {feature: "<name>", pick: "<the pick in the human\'s words>"} for the first design, and {feature: "<name>"} for a rerun (optional: use ("design-system" | "code": which one holds where the design system and the product\'s code differ; without it the run stops at such a difference and lists it in the app-level doc "Design system and code"), stories (paths of the user stories to limit the run to), steer (an instruction for this run), rounds (fix rounds after the review, default 1), models ({prepare, plan, system, draw, wire, review, fix, notes}; default: planning, the design system and reviewing on the session\'s model, the rest on sonnet), model (one for all), skills (the skills folder; default ~/.claude/skills)), from a folder linked to the feature\'s OmniState project (omni link). Only the feature\'s Spec doc is required; it reads the behaviour tree, the Project spec, the design system, the research and the layout options when they exist and says at its start what it found. Uses the skills ui-design and ux-design (with design-taste-frontend and emil-design-eng). Writes to OmniState only: the design doc "Design" and the html doc "Design notes" on the feature, tokens and components in the project\'s design system (which it creates when the project has none), assumptions as questions answered by default on the behaviour tree, and replies to comments. Work files go in a temp folder. To draw from a new pick, rename the doc "Design" and run with pick again.',
  phases: [
    { title: 'Prepare', detail: 'what exists: Spec, Project spec, behaviour tree, design system, research, layout options, an earlier design' },
    { title: 'Match', detail: 'the design system against the product\'s real style values in the code; where they differ the run changes neither and stops for the human to decide' },
    { title: 'Plan', detail: 'first design: the screens of the pick, where each user story happens, the components needed' },
    { title: 'System', detail: 'first design: the design system (created when missing), its tokens and components, a Components board' },
    { title: 'Screens', detail: 'first design: one agent per screen draws its normal state', model: 'sonnet' },
    { title: 'Outline', detail: 'first design: one agent per user story lists the boards it needs, then one agent merges the lists so that a state two stories reach is drawn once', model: 'sonnet' },
    { title: 'States', detail: 'first design: one agent per user story draws its states and overlays from the merged list and wires its interactions', model: 'sonnet' },
    { title: 'Rework', detail: 'rerun: what changed in the tree and which comments are open, then one agent per affected user story edits its boards in place and replies; comments about a token or a component go to one agent for the design system', model: 'sonnet' },
    { title: 'Wire', detail: 'interactions on shared screens, flows on the page, no dangling target, no unreachable board', model: 'sonnet' },
    { title: 'Check', detail: 'missing tokens and components added by one agent, a review per user story on board images, fixes, then the token check over the whole doc' },
    { title: 'Notes', detail: 'the Design notes doc: the last run, motion for the code, the table of boards and situations', model: 'sonnet' },
  ],
}

const A = args || {}
if (!A.feature) throw new Error('args.feature is required: the name of the OmniState feature to design')
const F = A.feature
const SK = A.skills || '~/.claude/skills'
const UI = `${SK}/ui-design`
const UXD = `${SK}/ux-design`
const BOARDS = `${UXD}/references/boards.md`
const TASTE = `The taste skill is at ${SK}/design-taste-frontend.`
const slug = (t) => String(t).toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'x'
const RUN = `design-${slug(F)}`
const ROUNDS = Math.max(1, A.rounds || 1)
const ONLY = Array.isArray(A.stories) && A.stories.length ? A.stories : null

// Drawing, fixing and bookkeeping run on a cheaper model than planning, the
// design system and reviewing. Override per role with args.models, or with
// args.model for every role.
const MODELS = { prepare: 'sonnet', draw: 'sonnet', wire: 'sonnet', fix: 'sonnet', notes: 'sonnet', ...(A.models || {}) }
const opt = (role, o) => {
  const model = A.model || MODELS[role]
  return model ? { ...o, model } : o
}
const omni = (role) => `omni --as ${RUN}/${role}`
const RULES = (role) => `Every omni command starts \`${omni(role)}\`; a command about this feature (doc create, doc list, behaviour, comment list) also takes \`--feature "${F}"\`. Work files go in the scratch folder only; change nothing in any repository.`

const s = (props, required) => ({ type: 'object', properties: props, required: required || Object.keys(props) })
const str = { type: 'string' }
const int = { type: 'integer' }
const bool = { type: 'boolean' }
const strs = { type: 'array', items: str }
const list = (items, extra) => ({ type: 'array', items, ...(extra || {}) })
const BOARD = s({ id: str, name: str })
const COVER = s({ node: str, nodeId: str, board: str, boardName: str, note: str }, ['node', 'board', 'note'])
const FLOW = s({ name: str, start: str })
const SCREEN_ACT = s({ board: str, layer: str, leadsTo: str, leadsToName: str, moment: str }, ['board', 'layer', 'moment'])

// ---- Prepare ---------------------------------------------------------------
phase('Prepare')
const prep = await agent(
  `Find what exists for the OmniState feature "${F}", so a design run can start. Read only; create nothing but the scratch folder and files in it. ${RULES('prepare')}

1. \`${omni('prepare')} link\` prints the project this folder is linked to: return its ID and name. If it prints no project, return linked false and stop.
2. Scratch folder: \`mktemp -d\`. Return its absolute path.
3. Spec: \`${omni('prepare')} doc list --feature "${F}" --pillar spec\`. Return the IDs of the feature's spec docs, and hasSpec true when at least one holds written content (\`${omni('prepare')} doc read <id>\`).
4. Project spec: \`${omni('prepare')} doc list --global --pillar spec\`. Return the ID of the app-level spec doc (the one titled "Project spec" when there are several), or "".
5. Behaviour tree: \`${omni('prepare')} --feature "${F}" behaviour list --json\`. Return nodeCount (0 when the feature has no tree or the command fails) and the user stories: the nodes at level 1, each with its path, its title and how many nodes sit below it.
6. When the feature has a behaviour tree: run the commands under "Fingerprints" in ${UI}/references/notes.md with the scratch folder from step 2, and save the lines the last one prints as <scratch>/fingerprints-start.tsv.
7. Design system: \`${omni('prepare')} doc list --global --pillar ui\`; a doc of kind design-system is the project's. Return its ID or "". Return as matchDoc the ID of the html doc titled "Design system and code" in that list, or "".
8. The feature's docs: \`${omni('prepare')} doc list --feature "${F}"\`. Return researchDoc (the html doc titled "Design research"), optionsDoc (the design doc titled "Layout options"), designDoc (the design doc titled "Design") and notesDoc (the html doc titled "Design notes"), each an ID or "". For designDoc, \`${omni('prepare')} doc read <id>\` and return designPage (the id of its first <Page>) and designBoards (how many <Board> tags it holds).
9. \`${omni('prepare')} config show | grep -E '^(server|workspace_id):'\` prints the server and the workspace ID: return both. Run it with that grep, so only those two lines are printed.`,
  opt('prepare', {
    label: 'prepare',
    effort: 'low',
    schema: s({
      linked: bool, projectId: str, projectName: str, scratch: str, server: str, workspaceId: str,
      hasSpec: bool, specDocs: strs, projectSpecDoc: str,
      nodeCount: int, stories: list(s({ path: str, title: str, nodes: int })),
      designSystemDoc: str, matchDoc: str, researchDoc: str, optionsDoc: str,
      designDoc: str, designPage: str, designBoards: int, notesDoc: str,
    }, ['linked', 'projectId', 'scratch', 'hasSpec', 'specDocs', 'nodeCount', 'stories']),
  }),
)
if (!prep || !prep.linked || !prep.projectId) throw new Error('This folder is not linked to an OmniState project, or omni is not signed in. Run `omni link <project>` (and `omni login` if needed), then run again.')
if (!prep.hasSpec) throw new Error(`The feature "${F}" has no Spec doc with content. Write its spec in OmniState first (omni-grill), then run again.`)

const S = prep.scratch
const P = prep.projectId
const TREE = prep.nodeCount > 0
const PRINTS = `${S}/fingerprints-start.tsv`
const allStories = prep.stories || []
const HAS_DESIGN = !!prep.designDoc && (prep.designBoards || 0) > 0
log(`Found: Spec (${prep.specDocs.length} doc${prep.specDocs.length === 1 ? '' : 's'}); Project spec: ${prep.projectSpecDoc ? 'yes' : 'none'}; behaviour tree: ${TREE ? `${allStories.length} user stories, ${prep.nodeCount} nodes` : 'none, designing from the Spec alone'}; design system: ${prep.designSystemDoc ? 'yes' : 'none, it will be created'}; research: ${prep.researchDoc ? 'yes' : 'none'}; layout options: ${prep.optionsDoc ? 'yes' : 'none'}; finished design: ${HAS_DESIGN ? `${prep.designBoards} boards, so this is a rerun` : 'none, so this is the first design'}`)
if (HAS_DESIGN && !prep.notesDoc) throw new Error(`The design doc "Design" (${prep.designDoc}) holds ${prep.designBoards} board${prep.designBoards === 1 ? '' : 's'}, but the feature has no "Design notes" doc, so an earlier first design did not finish. Resume that run if its ID is at hand (Workflow with resumeFromRunId). When the boards are complete and only the notes are missing, write the notes as ${UI}/references/notes.md describes. Otherwise set the unfinished doc aside with \`omni doc rename ${prep.designDoc} "Design, unfinished"\` and run again with pick.`)
const RERUN = HAS_DESIGN
if (!RERUN && !A.pick) throw new Error(`The feature "${F}" has no finished design yet, so the run needs the pick: args {feature: "${F}", pick: "<which layout option, in the human's words>"}. ${prep.optionsDoc ? 'The options are in the design doc "Layout options".' : 'There are no layout options yet: run design-options first, or describe the layout in pick.'}`)
if (RERUN && A.pick) log('A finished design exists, so pick is not used: this run reworks the existing boards. To draw from a new pick, rename the doc "Design" and run again.')

const behaviour = (role) => (TREE
  ? `The behaviour: the feature's behaviour tree. \`${omni(role)} --feature "${F}" behaviour list [<node>]\` prints the nodes; \`behaviour read <path>\` prints one with its rules and questions. The Spec doc${prep.specDocs.length === 1 ? '' : 's'} (${prep.specDocs.join(', ')}; \`doc read <id>\`) give${prep.specDocs.length === 1 ? 's' : ''} the background.`
  : `The behaviour: the feature has no behaviour tree, so the Spec doc${prep.specDocs.length === 1 ? '' : 's'} decide${prep.specDocs.length === 1 ? 's' : ''} it (${prep.specDocs.join(', ')}; read with \`${omni(role)} doc read <id>\`). Create no tree.`)
const inputs = (role) => [
  `Feature: "${F}", project ${P}. Scratch folder: ${S}.`,
  behaviour(role),
  prep.projectSpecDoc ? `The Project spec (doc ${prep.projectSpecDoc}) says who the users are and what the product is.` : '',
  prep.researchDoc ? `Research: the html doc ${prep.researchDoc}.` : '',
  A.steer ? `The human asks of this run: ${A.steer}` : '',
  RULES(role),
].filter(Boolean).join(' ')
const assume = (role) => (TREE
  ? `An assumption goes onto the tree, on the node it belongs to: \`${omni(role)} --feature "${F}" behaviour ask <node path> "<question>" --answer "<the choice>"\`. Return each one you added.`
  : 'Return each assumption you made; the design notes will list them.')
const ASSUMED = list(s({ node: str, question: str, answer: str }))
// Token and component writes replace whole trees, so agents that run side by
// side never write them; one agent does, after they have finished.
const ONE_WRITER = 'Other agents draw beside you, so tokens and components are not yours to write. Where the design system lacks a value, use its closest token and return the value under missingTokens (the value, its kind, and the board and layer that want it). Where it lacks a component for a repeated element, build the element from frames on tokens and return it under handBuilt.'
const SHARED = 'A board that other stories also use may be edited by another agent at the same moment: change single layers of it with set operations, and when a rev is refused, read the layer again and redo your change on what is there now.'

// ---- shared steps ----------------------------------------------------------
const wire = (DOC, PAGE, flows, acts, note, boards) => agent(
  `Do the page's part of "Step 4: Wire the prototype" of ${UI}/SKILL.md for the design doc ${DOC}, page ${PAGE}, with ${BOARDS} (Prototype) and ${UI}/references/motion.md. ${inputs('wire')}

${note}
Interactions that the agents drawing the stories could not write themselves, because the control sits on a shared board or leads to a board another agent drew (the board and the layer of the control, the board it leads to as an ID in leadsTo or as a name in leadsToName, the moment): ${JSON.stringify(acts)}
${boards && boards.length ? `The boards by name: ${JSON.stringify(boards)}` : `Find a board named in leadsToName with \`${omni('wire')} doc read ${DOC}\`.`}
Flows the page must have: ${JSON.stringify(flows)}
1. Write each interaction above on its layer as motion.md gives it for that moment, keeping the interactions the layer already has. A control has one target per trigger: the same link asked twice is written once, and when two stories ask the same control to lead to different boards, write the one that belongs to the story a person meets first and return the other under conflicts.
2. \`${omni('wire')} design flows --doc ${DOC} --page ${PAGE}\` prints the page's present flows. Give the page the present ones whose start board still exists, plus those above. Flow names are unique.
3. Run that command again. Fix each dangling target by pointing its interaction at the board that now shows that situation. For each unreachable board other than "Components": add the interaction on the control that leads there, or, when only an event outside the person's control leads there, a flow of its own as the skill says.
4. Repeat step 3 until it shows no dangling target and no unreachable board except "Components", three times at most.

Return how many flows the page has, the dangling targets and unreachable boards still left, and the conflicts.`,
  opt('wire', { label: 'wire', phase: 'Wire', effort: 'medium', schema: s({ flows: int, dangling: strs, unreachable: strs, conflicts: strs }, ['flows', 'dangling', 'unreachable']) }),
)

// one reviewer and one fixer per board: a board shared by stories goes to the first
const ownBoards = (units) => {
  const seen = new Set()
  return units.map((u) => ({ ...u, boards: u.boards.filter((b) => (seen.has(b.id) ? false : (seen.add(b.id), true))) })).filter((u) => u.boards.length)
}
const FINDING = s({ board: str, layer: str, severity: { type: 'number' }, rule: str, problem: str, fix: str }, ['board', 'severity', 'problem', 'fix'])
const check = (DOC, units) => pipeline(
  ownBoards(units),
  async (u) => {
    let left = []
    let found = 0
    let fixed = 0
    for (let round = 1; round <= ROUNDS; round++) {
      const review = await agent(
        `Review the finished design of one user story as someone who did not draw it: points 1 to 6 of "Step 5: Check" in ${UI}/SKILL.md, with ${UXD}/references/judging.md and ${UXD}/references/rules.md. ${TASTE} Review only; change no board. ${inputs(`review-${u.key}`)}

The story: ${u.title}${TREE && u.path ? ` (node ${u.path}; \`${omni(`review-${u.key}`)} --feature "${F}" behaviour list ${u.path}\` prints its situations)` : ''}.
Its boards in design doc ${DOC}: ${JSON.stringify(u.boards)}
${u.coverage && u.coverage.length ? `What its drawer says covers each node: ${JSON.stringify(u.coverage)}` : ''}
${round > 1 ? `This is the look after a fix round. The earlier findings: ${JSON.stringify(left)}. Report what is still wrong and anything the fixes broke.` : ''}
Look at each board as an image (${BOARDS}, Seeing; work files in ${S}/review-${u.key}-${round}) and read its tags where a finding needs the layer.

Return every finding of severity 2 or more: the board's ID, the layer's ID when it has one, the severity, the rule ID when a rule applies, the problem as the user meets it, and the fix.`,
        opt('review', { label: `review:${u.key}${round > 1 ? `:${round}` : ''}`, phase: 'Check', effort: 'high', schema: s({ findings: list(FINDING) }) }),
      )
      if (!review) { left = left.length ? left : [{ board: '', severity: 0, problem: 'the review returned nothing', fix: '' }]; break }
      if (round === 1) found = review.findings.length
      left = review.findings
      if (!left.length) break
      const fix = await agent(
        `Fix the boards of one user story for what a review found, as "Reworking a finished design" of ${UI}/SKILL.md says boards are changed (in place, tokens only), with ${BOARDS}. ${TASTE} ${inputs(`fix-${u.key}`)}

Design doc ${DOC}. Findings: ${JSON.stringify(left)}
Fix every finding of severity 3 or more, and each lighter one that is a single edit. Other agents fix other stories beside you, so tokens and components are not yours to write: a finding that needs a new token or a changed component is returned as not fixed, with what it needs. Look at the image of every board you change (work files in ${S}/fix-${u.key}-${round}).

Return the findings you fixed and those you did not, each in one line.`,
        opt('fix', { label: `fix:${u.key}${round > 1 ? `:${round}` : ''}`, phase: 'Check', effort: 'medium', schema: s({ fixed: strs, notFixed: strs }) }),
      )
      if (!fix) break
      fixed += fix.fixed.length
      if (round === ROUNDS) { left = fix.notFixed.map((t) => ({ board: '', severity: 0, problem: t, fix: '' })); break }
    }
    return { key: u.key, title: u.title, found, fixed, open: left.map((f) => `${f.board ? `${f.board}: ` : ''}${f.problem}`) }
  },
)

const SCAN = s({ colours: list(s({ attribute: str, value: str, count: int })), measures: list(s({ text: str, count: int })) })
const scanOnce = (DOC, DS, n) => agent(
  `Run the token check of "Step 5: Check" in ${UI}/SKILL.md: its commands, exactly as written there, with \`${omni('scan')}\`, the design doc ${DOC}, the design system doc ${DS || '(find it with `doc list --global --pillar ui`)'} and the scratch folder ${S}/scan-${n}. Change nothing. Return every line of the first grep as a literal colour (the attribute name before the = or :, the colour, the count) and every line of the second as a literal measure (the matched text, the count). Empty lists when the greps print nothing.`,
  opt('prepare', { label: `token-check${n > 1 ? `:${n}` : ''}`, phase: 'Check', effort: 'low', schema: SCAN }),
)
const rawOf = (r) => ({
  colours: r.colours.filter((c) => !/^(background|content)$/i.test(c.attribute)),
  measures: r.measures.filter((m) => !/^[A-Za-z]+=\{0\}$/.test(m.text.trim())),
})
const tokenCheck = async (DOC, DS) => {
  let r = await scanOnce(DOC, DS, 1)
  if (!r) return { ran: false, colours: [], measures: [] }
  let raw = rawOf(r)
  if (raw.colours.length || raw.measures.length) {
    log(`Token check: ${raw.colours.length} literal colours and ${raw.measures.length} literal measures; replacing them with tokens`)
    await agent(
      `Replace every literal value the token check found with a token, as "Tokens only" and the end of "Step 5: Check" in ${UI}/SKILL.md say, with ${BOARDS} for changing boards. No other agent is working now: you are the one writer of tokens and components. ${inputs('fix-tokens')}

Design doc ${DOC}; design system doc ${DS || '(find it with `doc list --global --pillar ui`)'}; the workspace ID is ${prep.workspaceId || 'not known'}.
Literal colours: ${JSON.stringify(raw.colours)}
Literal measures: ${JSON.stringify(raw.measures)}
Find each in the doc (\`${omni('fix-tokens')} doc read ${DOC}\`) or in the components, use the token that holds that value, and add the token first when the system lacks one. Run the check's commands yourself until they list nothing but what may stay.`,
      opt('fix', { label: 'fix:tokens', phase: 'Check', effort: 'medium' }),
    )
    r = await scanOnce(DOC, DS, 2)
    if (r) raw = rawOf(r)
  }
  return { ran: true, ...raw }
}
const tokenLine = (t) => `Token check: ${!t.ran ? 'did not run' : t.colours.length || t.measures.length ? `${t.colours.length} literal colours and ${t.measures.length} literal measures are left` : 'no literal colour or measure is left'}`

const complete = (DOC, DS, handBuilt, missingTokens, scope) => agent(
  `Agents that drew boards of the design doc ${DOC} side by side could not write tokens or components. Add what they lacked: "Tokens only" and the Components part of "Step 1: The design system" in ${UI}/SKILL.md, with ${BOARDS}. No other agent is working now: you are the one writer. ${inputs('system')}

Design system doc ${DS || '(find it with `doc list --global --pillar ui`)'}; the workspace ID is ${prep.workspaceId || 'not known'}.
Values the system lacked, with the layers that want them: ${JSON.stringify(missingTokens)}
Elements built from frames for lack of a component: ${JSON.stringify(handBuilt)}
For each value: add the token, then set it on the layers named. For each element that appears more than once: define the component (sending every existing component with its id in the same PUT), replace the frames on the boards with instances of it, keeping each layer's interactions on the instance, and add it to the board "Components". ${scope} Look at the images of the boards you changed (work files in ${S}/complete).

Return the tokens you added, the components you added, and the elements you left as frames with the reason.`,
  opt('system', { label: 'tokens-and-components', phase: 'Check', effort: 'medium', schema: s({ tokens: strs, components: strs, left: strs }) }),
)

const notes = async (DOC, data) => {
  const once = (n) => agent(
    `Do "Step 6: The design notes" of ${UI}/SKILL.md for the feature "${F}", following ${UI}/references/notes.md and, for its motion part, ${UI}/references/motion.md. ${inputs('notes')}

The design doc is ${DOC}. ${prep.notesDoc ? `The notes doc is ${prep.notesDoc}: update it as notes.md says, changing only what changed.` : 'There is no notes doc yet: create it.'}
${TREE
    ? `Rows: write or update a row for each node in "coverage" below, and only for those; every other row stays exactly as it is. The fingerprints taken when this run started are in ${PRINTS} (node ID, path, fingerprint): a covered node's row gets that one. Only for a node this run itself put a question on (the nodes named in "assumptions" below), compute the fingerprint as it is now and write that.`
    : 'The feature has no behaviour tree: one row per board, and the part "Assumed".'}
A coverage entry with a board name and no board ID is shown by a board another story drew: take that board's ID from the boards listed below, by its name.
What this run did: ${JSON.stringify(data)}

Return the notes doc's ID.`,
    opt('notes', { label: `notes${n > 1 ? `:${n}` : ''}`, phase: 'Notes', effort: 'medium', schema: s({ doc: str }) }),
  )
  const first = await once(1)
  if (first && first.doc) return first.doc
  const second = await once(2)
  if (second && second.doc) return second.doc
  log(`The design notes were not written, after two tries. ${prep.notesDoc ? 'The table still shows the state before this run, so the next run takes up the same changes again.' : `The next run stops until they exist: have a session write them as ${UI}/references/notes.md describes, from this run's result.`}`)
  return prep.notesDoc || ''
}
const link = (doc) => (prep.server && prep.workspaceId ? `${prep.server}/w/${prep.workspaceId}/projects/${P}/docs/${doc}` : '')

// ---- Match -----------------------------------------------------------------
// Boards take every value from the design system, so it has to describe the
// product as it is. Where it differs from the product's code, the human decides
// which one holds: the run changes neither and stops, unless args.use says.
phase('Match')
const USE = A.use === 'code' || A.use === 'design-system' ? A.use : ''
const match = await agent(
  `Do "The design system and the product's code" of ${UI}/SKILL.md for the feature "${F}". Change no token, no component and no code. ${inputs('match')}

The product's code, when there is any, is in the folder you run in, the one linked to the project.
${prep.designSystemDoc
    ? `The project's design system is doc ${prep.designSystemDoc}; \`${omni('match')} tokens export\` prints its tokens. When they differ, write the differences to the doc the skill names${prep.matchDoc ? ` (it exists: doc ${prep.matchDoc}; read it and replace its content with the present comparison)` : ''}. When they agree${prep.matchDoc ? `, replace that doc's content with one line saying so and what was compared` : ', write nothing'}.`
    : 'The project has no design system yet: only find the product\'s style values and say where they are, so the new system can be built from them. Write nothing.'}

Return codeStyles (true when the folder holds the product's style values), the files they are in, the differences (the role, the token's path, the design system's value, the code's value with its name there; one entry per role and mode), how many roles agree, and the comparison doc's ID or "".`,
  opt('match', {
    label: 'match',
    effort: 'medium',
    schema: s({
      codeStyles: bool, files: strs, agree: int, doc: str,
      differences: list(s({ role: str, token: str, designSystem: str, code: str })),
    }, ['codeStyles', 'files', 'agree', 'differences']),
  }),
)
if (!match) throw new Error('The check of the design system against the product\'s code returned nothing, so nothing was drawn. Run again.')
const differs = prep.designSystemDoc ? match.differences.length : 0
if (!match.codeStyles) log('Match: this folder holds no style values of the product, so there is nothing to compare the design system with.')
else if (!prep.designSystemDoc) log(`Match: the project has no design system; the new one is built from the product's own style values (${match.files.join(', ')}).`)
else if (!differs) log(`Match: the design system agrees with the product's code in all ${match.agree} roles compared.`)
else if (!USE) {
  log(`Match: the design system and the product's code differ in ${differs} place${differs === 1 ? '' : 's'} (${match.differences.slice(0, 3).map((d) => `${d.role}: ${d.designSystem} against ${d.code}`).join('; ')}${differs > 3 ? '; ...' : ''}). Nothing was changed and nothing was drawn: which one holds is the human's decision.`)
  return {
    feature: F,
    stopped: 'The design system and the product\'s code differ',
    differences: match.differences.slice(0, 60),
    differencesInAll: differs,
    agree: match.agree,
    files: match.files,
    doc: match.doc || prep.matchDoc || '',
    link: link(match.doc || prep.matchDoc || ''),
    next: `The human decides which one holds, then runs design-feature again with the same args plus use: "design-system" (draw with the design system as it is) or use: "code" (set the differing tokens to the code's values first, which every board of the project follows). Correcting either side by hand and running again works too.`,
  }
} else if (USE === 'design-system') {
  log(`Match: the design system and the product's code differ in ${differs} place${differs === 1 ? '' : 's'}; the human chose the design system, so it is used as it is.`)
} else {
  const taken = await agent(
    `The human decided: where the project's design system and the product's code differ, the code holds. Set each token below to the code's value, as "Tokens only" of ${UI}/SKILL.md says tokens are written (a colour once per mode). No other agent is working now: you are the one writer of tokens. Change only these tokens. ${inputs('system')}

Design system doc ${prep.designSystemDoc}. The differences: ${JSON.stringify(match.differences)}
${match.doc || prep.matchDoc ? `Then add one line at the top of the doc ${match.doc || prep.matchDoc}: that the human chose the code and the tokens were set to it in this run.` : ''}

Return the tokens you changed and those you could not change, with the reason.`,
    opt('system', { label: 'take-code', effort: 'medium', schema: s({ changed: strs, notChanged: strs }) }),
  )
  log(`Match: the human chose the code; ${taken ? `${taken.changed.length} token${taken.changed.length === 1 ? '' : 's'} set to its values${taken.notChanged.length ? `, ${taken.notChanged.length} could not be set: ${taken.notChanged.join('; ')}` : ''}` : 'but the agent that sets the tokens returned nothing, so the design system is unchanged'}.`)
}

// ---- Rerun -----------------------------------------------------------------
if (RERUN) {
  const DOC = prep.designDoc
  const PAGE = prep.designPage
  const DS = prep.designSystemDoc || ''
  phase('Rework')
  const diff = await agent(
    `Find what a rerun of the finished design of the feature "${F}" has to change. Read only. ${inputs('diff')}

The design doc is ${DOC}; the notes doc is ${prep.notesDoc}.
1. The boards: \`${omni('diff')} doc read ${DOC}\`. Return the width and height of the screens' boards (the size most boards share), and lowestY: the largest y + height of any board.
2. Open comments: \`${omni('diff')} comment list --doc ${DOC} --json\`; a comment is open while waiting_on_you is false. For each, find its board as ${BOARDS} (Comments) says, and the user story the board belongs to: the first part of the node path on the first row of the notes table that names that board ("" when no row names it).
${TREE
    ? `3. Changes in the tree: "What a rerun reads from it" in ${UI}/references/notes.md. The fingerprints as they are now are already in ${PRINTS} (node ID, path, fingerprint, one node per line): compare them with the notes table. Return the changed nodes with the boards of their rows, the new nodes, and the gone ones. A node's story is the first part of its path; the root's path is ".".`
    : '3. The feature has no behaviour tree: return no changed, new or gone nodes.'}

Return these, and a one-line note on anything that could not be read.`,
    opt('prepare', {
      label: 'what-changed',
      effort: 'medium',
      schema: s({
        width: int, height: int, lowestY: int,
        comments: list(s({ id: str, node: str, board: str, boardName: str, story: str, text: str })),
        changed: list(s({ node: str, nodeId: str, story: str, boards: strs })),
        added: list(s({ node: str, nodeId: str, story: str })),
        gone: list(s({ node: str, board: str })),
        note: str,
      }, ['width', 'height', 'lowestY', 'comments', 'changed', 'added', 'gone']),
    }),
  )
  if (!diff) throw new Error('Could not read what changed since the last run')
  // the root describes the whole feature: a change there names no board
  const isRoot = (n) => n.node === '.' || n.node === '' || n.story === '.'
  const rootChanged = [...diff.changed, ...diff.added].some(isRoot)
  const inScope = (st) => !ONLY || ONLY.includes(st)
  const groups = {}
  const group = (st) => (groups[st || ''] = groups[st || ''] || { story: st || '', comments: [], changed: [], added: [] })
  diff.comments.filter((c) => !c.story || inScope(c.story)).forEach((c) => group(c.story).comments.push(c))
  diff.changed.filter((c) => !isRoot(c) && inScope(c.story)).forEach((c) => group(c.story).changed.push(c))
  diff.added.filter((c) => !isRoot(c) && inScope(c.story)).forEach((c) => group(c.story).added.push(c))
  const general = groups[''] || null
  const units = Object.values(groups).filter((g) => g.story)
  const skipped = ONLY ? [...diff.changed, ...diff.added].filter((c) => !isRoot(c) && !inScope(c.story)).length : 0
  log(`Since the last run: ${diff.comments.length} open comment${diff.comments.length === 1 ? '' : 's'}, ${diff.changed.length} changed, ${diff.added.length} new and ${diff.gone.length} gone situation${diff.gone.length === 1 ? '' : 's'}${diff.note ? ` (${diff.note})` : ''}${ONLY ? `; limited to ${ONLY.join(', ')}, so ${skipped} change${skipped === 1 ? '' : 's'} in other stories wait for a later run` : ''}`)
  if (rootChanged) log('The root node of the tree changed. It describes the whole feature and names no board, so no board follows from it alone: say in steer what should change.')
  if (!units.length && !general) {
    log('Nothing to change: no open comments and no changed situations. Every board stays as it is.')
    return { feature: F, mode: 'rerun', doc: DOC, link: link(DOC), changed: [], gone: diff.gone, note: 'Nothing to change' }
  }

  const reworked = (await parallel(units.map((g, i) => () => {
    const key = slug(g.story)
    const title = (allStories.find((x) => x.path === g.story) || {}).title || g.story
    return agent(
      `Rework the finished boards of one user story, as "Reworking a finished design" of ${UI}/SKILL.md says: steps 2 and 3 for what is new, in place for what exists, and the story's part of step 4. Use ${BOARDS} and ${UI}/references/motion.md. ${TASTE} ${inputs(`story-${key}`)}

The story: ${title} (node ${g.story}). Design doc ${DOC}, page ${PAGE}; \`${omni(`story-${key}`)} doc read ${DOC}\` prints every board, and the notes doc ${prep.notesDoc} says which board shows which node.
The design system${DS ? ` (doc ${DS}; \`doc read\` prints its tokens and then its components)` : ''}: \`${omni(`story-${key}`)} tokens export\` prints its tokens.
Open comments to act on and reply to: ${JSON.stringify(g.comments)}
Situations that changed in the tree, with their boards: ${JSON.stringify(g.changed)}
Situations that are new in the tree: ${JSON.stringify(g.added)}
Touch only the boards these name, and the new boards they need. Before drawing a new board, look in the notes table and the doc for a board that already shows that state of that screen, in any story: when there is one, point the node to it instead of drawing it again. A new board is ${diff.width} wide and ${diff.height} high (an overlay is as large as its content) and sits right of the story's last board, in its row; when the story has no board yet, its row starts at x 0, y ${diff.lowestY + 240 + i * (diff.height + 240)}. ${assume(`story-${key}`)}
${ONE_WRITER} A comment that can only be answered by changing a token or a component is not replied to: return it under forSystem with what it asks.
${SHARED} An interaction on the normal board of a screen is not yours to write either: return it under screenInteractions (the board's ID, the layer's ID, the ID of the board it leads to, and the moment in the words of motion.md's table).
Look at the image of every board you change or add (work files in ${S}/story-${key}).

Return the boards you changed and the boards you added; for every changed or new node, the board that shows it now (or "" with the reason in note, also when the change called for nothing on the board); each comment you replied to with the reply; the comments for the design system; the screen interactions; the flows the new boards need; missing tokens; what you built by hand; assumptions; and anything that still fails.`,
      opt('draw', {
        label: `rework:${key}`,
        phase: 'Rework',
        effort: 'high',
        schema: s({
          changedBoards: list(BOARD), addedBoards: list(BOARD), coverage: list(COVER),
          replies: list(s({ comment: str, reply: str })), forSystem: list(s({ comment: str, asks: str })),
          screenInteractions: list(SCREEN_ACT), flows: list(FLOW),
          missingTokens: strs, handBuilt: strs, assumptions: ASSUMED, failing: strs,
        }, ['changedBoards', 'addedBoards', 'coverage', 'replies', 'forSystem', 'screenInteractions', 'flows', 'missingTokens', 'handBuilt', 'failing']),
      }),
    ).then((r) => (r ? { ...r, key, story: g.story, title } : null))
  }))).filter(Boolean)
  if (units.length && !reworked.length) throw new Error('No rework agent returned; the boards are as they were')
  const lost = units.filter((g) => !reworked.some((r) => r.story === g.story)).map((g) => g.story)
  if (lost.length) log(`Not reworked, their agents returned nothing: ${lost.join(', ')}. Their changes stay marked as unseen and come up again on the next run.`)

  const forSystem = [...(general ? general.comments.map((c) => ({ comment: c.id, node: c.node, board: c.boardName || c.board, asks: c.text })) : []), ...reworked.flatMap((r) => r.forSystem)]
  const systemRework = forSystem.length ? await agent(
    `Answer comments on the finished design of the feature "${F}" that ask for a change in the design system or on a board outside any user story, as "Reworking a finished design", "Tokens only" and the Components part of "Step 1: The design system" in ${UI}/SKILL.md say, with ${BOARDS}. No other agent is working now: you are the one writer of tokens and components. ${inputs('system')}

Design doc ${DOC}; design system doc ${DS || '(find it with `doc list --global --pillar ui`)'}; the workspace ID is ${prep.workspaceId || 'not known'}.
The comments: ${JSON.stringify(forSystem)}
A change here shows on every board that uses the token or the component: look at the image of the board "Components" and of two boards that use what you changed (work files in ${S}/system). Reply to each comment with what changed.

Return the boards you edited directly, each comment with your reply, and anything that still fails.`,
    opt('system', {
      label: 'rework:design-system',
      phase: 'Rework',
      effort: 'high',
      schema: s({ changedBoards: list(BOARD), replies: list(s({ comment: str, reply: str })), failing: strs }),
    }),
  ) : null

  const wired = await wire(DOC, PAGE, reworked.flatMap((r) => r.flows), reworked.flatMap((r) => r.screenInteractions), 'This is a rerun: keep every flow the page has, and add the ones below for boards that are new.')

  phase('Check')
  const handBuilt = reworked.flatMap((r) => r.handBuilt)
  const missingTokens = reworked.flatMap((r) => r.missingTokens)
  const touched = reworked.flatMap((r) => [...r.changedBoards, ...r.addedBoards])
  const completed = handBuilt.length || missingTokens.length
    ? await complete(DOC, DS, handBuilt, missingTokens, `Change only these boards, the ones this run touched: ${JSON.stringify(touched)}.`)
    : null
  const checked = (await check(DOC, reworked.map((r) => ({ key: r.key, path: r.story, title: r.title, boards: [...r.changedBoards, ...r.addedBoards], coverage: r.coverage })))).filter(Boolean)
  const tokens = await tokenCheck(DOC, DS)
  log(tokenLine(tokens))

  phase('Notes')
  const summary = {
    mode: 'rerun',
    changedBoards: reworked.flatMap((r) => r.changedBoards),
    addedBoards: reworked.flatMap((r) => r.addedBoards),
    coverage: reworked.flatMap((r) => r.coverage),
    comments: [...reworked.flatMap((r) => r.replies), ...(systemRework ? systemRework.replies : [])],
    designSystemChanges: systemRework ? systemRework.changedBoards : [],
    gone: diff.gone,
    notReworked: lost,
    waitingInOtherStories: skipped,
    assumptions: reworked.flatMap((r) => r.assumptions || []),
    review: checked,
    tokensAdded: completed ? completed.tokens : [],
    componentsAdded: completed ? completed.components : [],
    leftAsFrames: completed ? completed.left : handBuilt,
    wiring: wired || { flows: 0, dangling: ['the wiring step returned nothing'], unreachable: [] },
    tokenCheck: tokens,
    failing: [...reworked.flatMap((r) => r.failing), ...(systemRework ? systemRework.failing : [])],
  }
  const notesDoc = await notes(DOC, summary)
  return { feature: F, doc: DOC, link: link(DOC), notesDoc, ...summary }
}

// ---- First design: Plan ----------------------------------------------------
phase('Plan')
const plan = await agent(
  `Plan the finished design of the feature "${F}": the screens, and where each user story happens. This is the first part of "Step 2: Plan the boards" of ${UI}/SKILL.md; the states of each story are planned later, by the agent that draws that story. Plan only; draw nothing. ${inputs('plan')}

The human's pick: ${A.pick}
${prep.optionsDoc ? `The layout options are in the design doc ${prep.optionsDoc}: read it (\`${omni('plan')} doc read ${prep.optionsDoc}\`), read the board "Comparison", and look at the images of the boards the pick names (${BOARDS}, Seeing; work files in ${S}/plan). The human may have commented on them: \`${omni('plan')} comment list --doc ${prep.optionsDoc} --json\`; take each open comment (waiting_on_you false) into the plan and reply to it with how the finished design takes it up.` : 'There are no layout options: the pick itself describes the layout.'}
${TREE
    ? `The user stories: ${allStories.map((x) => `${x.path} (${x.title}; ${x.nodes} nodes below)`).join('; ')}.${ONLY ? ` This run draws only: ${ONLY.join(', ')}. Plan the screens those stories use.` : ''}`
    : `Derive the jobs from the Spec; each job is a story here, with a short path of lowercase words joined by -.${ONLY ? ` This run draws only the jobs the human names as ${ONLY.join(', ')}: return those, with exactly these paths.` : ''}`}
${assume('plan')}

Return: the screen's width and height in points; the screens (a key of lowercase words joined by -, a name, the layout in a few lines, and in "from" the IDs of the option boards it comes from, or "" when there are none); per story its path, its title, the keys of the screens it uses and the key of the screen where it begins; and the components the screens repeat (a name and what it is), whether the design system has them or not.`,
  opt('plan', {
    label: 'plan',
    effort: 'high',
    schema: s({
      width: int, height: int,
      screens: list(s({ key: str, name: str, layout: str, from: str }), { minItems: 1 }),
      stories: list(s({ path: str, title: str, screens: strs, entry: str }), { minItems: 1 }),
      components: list(s({ name: str, what: str })),
      assumptions: ASSUMED,
    }),
  }),
)
if (!plan) throw new Error('The plan step returned nothing')
const W = plan.width
const H = plan.height
const GAP = 160
const ROW = H + 240
const planStories = ONLY ? plan.stories.filter((x) => ONLY.includes(x.path)) : plan.stories
if (!planStories.length) throw new Error(`None of the stories in args.stories (${(ONLY || []).join(', ')}) is in the plan: ${plan.stories.map((x) => x.path).join(', ')}`)
const usedKeys = new Set(planStories.flatMap((x) => [...x.screens, x.entry]))
const screens = plan.screens.filter((sc) => !ONLY || usedKeys.has(sc.key))
log(`Plan: ${screens.length} screen${screens.length === 1 ? '' : 's'} at ${W}×${H}, ${planStories.length} user stor${planStories.length === 1 ? 'y' : 'ies'}, ${plan.components.length} component${plan.components.length === 1 ? '' : 's'}, ${plan.assumptions.length} assumption${plan.assumptions.length === 1 ? '' : 's'}`)

const page = await agent(
  `Make the page that the finished design of the feature "${F}" will be drawn on. ${RULES('page')}
${prep.designDoc
    ? `The design doc ${prep.designDoc} exists and is empty: name its page ${prep.designPage} "Design", as ${BOARDS} (Changing a board that exists) describes.`
    : `Create the design doc: \`${omni('page')} doc create --feature "${F}" --kind design --pillar ui --title "Design"\`. It starts with one empty page: read it (\`${omni('page')} doc read <doc id>\`) and name that page "Design" with [{"op":"set","node":"<page id>","rev":1,"properties":{"name":"Design"}}] sent to \`${omni('page')} design edit <doc id> --body-file -\`.`}
Return the doc's ID and the page's ID.`,
  opt('prepare', { label: 'page', phase: 'Plan', effort: 'low', schema: s({ doc: str, page: str }) }),
)
if (!page || !page.doc || !page.page) throw new Error('The page for the design could not be made')
const DOC = page.doc
const PAGE = page.page

// ---- System ----------------------------------------------------------------
phase('System')
const system = await agent(
  `Do "Step 1: The design system" of ${UI}/SKILL.md, all of it, for the feature "${F}", with "Tokens only" of the same file, ${UI}/references/motion.md for the motion tokens and ${BOARDS} for the Components board. ${TASTE} No other agent is working now: you are the one writer of tokens and components. ${inputs('system')}

${prep.designSystemDoc ? `The project has a design system (doc ${prep.designSystemDoc}): keep its look, and add only the tokens and components the plan needs and it lacks.` : match.codeStyles ? `The project has no design system: create it from the product's own style values, which are in ${match.files.join(', ')} in the folder you run in, as the skill says for that case.` : 'The project has no design system and the folder holds no style values of the product: create it from a style, as the skill says.'}
The plan's screens: ${JSON.stringify(screens.map((sc) => ({ name: sc.name, layout: sc.layout })))}
Components the screens repeat: ${JSON.stringify(plan.components)}
The human's pick: ${A.pick}
The workspace ID is ${prep.workspaceId || 'not known'}. Work files (the style sheet, tokens, components) go in ${S}/system.
Draw the board "Components" in design doc ${DOC}, page ${PAGE}, at x ${-(960 + GAP)}, y 0, 960 wide, height "hug", and look at its image.

Return the design system doc's ID, whether you created it, the style and the one line on why (when created), every component (its ID, its name and its props with their values, in one line each), the token groups with their paths, the Components board's ID, and anything that still fails.`,
  opt('system', {
    label: 'design-system',
    effort: 'high',
    schema: s({
      doc: str, created: bool, style: str, why: str,
      components: list(s({ id: str, name: str, props: str })),
      tokens: strs, board: str, failing: strs,
    }, ['doc', 'created', 'components', 'tokens', 'failing']),
  }),
)
if (!system || !system.doc) throw new Error(`The design system step returned nothing; no board was drawn. The design doc ${DOC} is still empty, so the same args start over.`)
const DS = system.doc
const systemFor = (role) => `The design system (doc ${DS}): \`${omni(role)} tokens export\` prints its tokens. Token paths by group: ${system.tokens.join(' | ')}. Components: ${JSON.stringify(system.components)}. ${ONE_WRITER}`

// ---- Screens ---------------------------------------------------------------
phase('Screens')
const screenBoards = await parallel(screens.map((sc, i) => () => agent(
  `Draw one screen of the finished design of the feature "${F}" in its normal state: "Step 3: Draw" of ${UI}/SKILL.md for this one board, with "Tokens only" of the same file and ${BOARDS}. ${TASTE} ${inputs(`screen-${sc.key}`)}

The screen: ${JSON.stringify(sc)}
${prep.optionsDoc && sc.from ? `It comes from the layout options in design doc ${prep.optionsDoc}; read the boards named in "from" (\`${omni(`screen-${sc.key}`)} design read --doc ${prep.optionsDoc} --board <id>\`) for the structure and the copy.` : ''}
The human's pick: ${A.pick}
The other screens of the design, drawn by other agents at the same time: ${screens.filter((o) => o.key !== sc.key).map((o) => o.name).join(', ') || 'none'}. Parts that all screens share (navigation, top bar) follow the plan's layout and use the same components, so they match.
${systemFor(`screen-${sc.key}`)}

Draw into design doc ${DOC}, page ${PAGE}: one board named "${sc.name}", ${W} wide and ${H} high, at x ${i * (W + GAP)}, y 0. Work files go in ${S}/screen-${sc.key}.

Return the board's ID and name, missing tokens, what you built by hand, and anything that still fails after three looks.`,
  opt('draw', {
    label: `screen:${sc.key}`,
    phase: 'Screens',
    effort: 'medium',
    schema: s({ id: str, name: str, missingTokens: strs, handBuilt: strs, failing: strs }),
  }),
).then((r) => (r && r.id ? { ...r, key: sc.key } : null))))
const drawnScreens = screenBoards.filter(Boolean)
if (!drawnScreens.length) log('No screen was drawn in its normal state; each story draws the screens it needs.')
else if (drawnScreens.length < screens.length) log(`Screens not drawn: ${screens.filter((sc) => !drawnScreens.some((d) => d.key === sc.key)).map((sc) => sc.name).join(', ')}`)

// ---- Outline ---------------------------------------------------------------
// Each story lists the boards it needs before anyone draws, and one agent merges
// the lists: stories overlap, and a state two of them reach is drawn once.
phase('Outline')
const OUT_BOARD = s({ name: str, screen: str, state: str, sees: str, overlay: bool, nodes: strs })
const NO_BOARD = s({ node: str, shownBy: str, note: str }, ['node', 'note'])
const screenList = JSON.stringify(drawnScreens.map((d) => ({ key: d.key, board: d.id, name: d.name })))
const outlines = (await parallel(planStories.map((st) => () => {
  const key = slug(st.path)
  return agent(
    `List the boards one user story of the finished design of the feature "${F}" needs: its part of "Step 2: Plan the boards" of ${UI}/SKILL.md, with the situation checklist it names. List only; draw nothing. ${inputs(`outline-${key}`)}

The story: ${st.title} (${TREE ? `node ${st.path}; \`${omni(`outline-${key}`)} --feature "${F}" behaviour list ${st.path}\` prints every situation below it, and \`behaviour read <path>\` one of them in full` : `a job from the Spec, path ${st.path}; give each situation you derive a path under it`}).
The screens, already drawn in their normal state: ${screenList}. This story uses ${st.screens.join(', ')} and begins on ${st.entry}.
${assume(`outline-${key}`)}

Return the boards the story needs beyond the screens' normal boards: for each a name as the skill names boards, the key of its screen, the state in two or three words, what the person sees there in one line, whether it is an overlay, and the paths of the nodes it shows. And every node of the story, the story's own node included, that needs no board of its own: the node, in shownBy the key of the screen whose normal board shows it (or ""), and the reason in note (same as another node, with that node's path; the outcome is off screen). Also the assumptions.`,
    opt('draw', {
      label: `outline:${key}`,
      phase: 'Outline',
      effort: 'medium',
      schema: s({ boards: list(OUT_BOARD), noBoard: list(NO_BOARD), assumptions: ASSUMED }, ['boards', 'noBoard']),
    }),
  ).then((r) => (r ? { ...r, key, path: st.path, title: st.title, screens: st.screens, entry: st.entry } : null))
}))).filter(Boolean)
const missing = planStories.filter((st) => !outlines.some((o) => o.path === st.path))
if (missing.length) log(`Stories not outlined, so not drawn: ${missing.map((m) => m.path).join(', ')}`)

const merged = outlines.length > 1 ? await agent(
  `Agents each listed the boards one user story of the finished design of the feature "${F}" needs, without seeing each other's lists. Stories overlap: two of them often reach the same state of the same screen. Apply "One board per state" of "Step 2: Plan the boards" in ${UI}/SKILL.md to the lists. Change nothing else in them; draw nothing.

The lists, in the order a person meets the stories: ${JSON.stringify(outlines.map((o) => ({ story: o.path, title: o.title, boards: o.boards })))}

Two boards are the same when the person sees the same thing on the same screen, whatever the two lists call it. Keep one, in the story a person meets first, give it the nodes of both, and take the other out of its story's list. Then make every board name unique across all lists.

Return, for every story by its path: its boards after this (each with all the nodes it now shows), and under borrowed the boards kept in another story that show nodes of this one (the board's name, the story that draws it, and this story's nodes it shows). And each merge and each renaming in one line.`,
  opt('plan', {
    label: 'one-board-per-state',
    phase: 'Outline',
    effort: 'medium',
    schema: s({
      stories: list(s({ path: str, boards: list(OUT_BOARD), borrowed: list(s({ name: str, from: str, nodes: strs })) })),
      merges: strs,
    }),
  }),
) : null
if (outlines.length > 1 && !merged) log('The lists of the stories were not merged, so a state that two stories reach may be drawn twice.')
if (merged) log(`One board per state: ${merged.merges.length ? merged.merges.join('; ') : 'no state is reached by two stories'}`)
const listFor = (o) => {
  const m = merged && merged.stories.find((x) => x.path === o.path)
  return m ? { boards: m.boards, borrowed: m.borrowed } : { boards: o.boards, borrowed: [] }
}

// ---- States ----------------------------------------------------------------
phase('States')
const storyResults = (await parallel(outlines.map((o, j) => () => {
  const key = o.key
  const mine = listFor(o)
  return agent(
    `Draw one user story of the finished design of the feature "${F}": "Step 3: Draw" and the story's part of "Step 4: Wire the prototype" of ${UI}/SKILL.md, with "Tokens only" of the same file, ${BOARDS} and ${UI}/references/motion.md. ${TASTE} ${inputs(`story-${key}`)}

The story: ${o.title} (${TREE ? `node ${o.path}; \`${omni(`story-${key}`)} --feature "${F}" behaviour read <path>\` prints a situation in full` : `a job from the Spec, path ${o.path}`}).
The screens, already drawn in their normal state in design doc ${DOC}, page ${PAGE}: ${screenList}. This story uses ${o.screens.join(', ')} and begins on ${o.entry}. A screen of the story that is missing from that list was not drawn: when this story begins on it, draw its normal state as the first board of your row, from this plan: ${JSON.stringify(screens.filter((sc) => o.screens.includes(sc.key) || sc.key === o.entry).map((sc) => ({ key: sc.key, name: sc.name, layout: sc.layout })))}.
Draw exactly these boards, with these names: ${JSON.stringify(mine.boards)}
These states of your story are drawn by another story's agent at this moment, so they are not yours to draw: ${JSON.stringify(mine.borrowed)}
Nodes that need no board of their own: ${JSON.stringify(o.noBoard)}
${systemFor(`story-${key}`)}
${assume(`story-${key}`)}

Each new board is ${W} wide and ${H} high (an overlay is as large as its content), at y ${(j + 1) * ROW}, the first at x 0 and each next one ${W + GAP} further right. Work files go in ${S}/story-${key}.
Interactions between your own boards you write yourself. Two kinds are written by another agent, from what you return under links: a control on a screen's normal board (those boards are shared with other stories and are not yours to write), and a control of yours that leads to a board another story draws. For each: the ID of the board the control is on, the layer's ID (or its name on a shared board), the target as a board ID in leadsTo or as the board's name in leadsToName, and the moment in the words of motion.md's table.
Flows are set on the page by another agent: return the ones this story needs (the story's own, starting at the board where it begins, and one per situation the main path cannot reach).

Return the boards you drew; for every node of the story, the story's own node included, the board that shows it (its ID and name; a screen's normal board counts; for a state another story draws, the board's name with an empty ID), or "" with the reason in note; the links; the flows; missing tokens; what you built by hand; assumptions; and anything that still fails after three looks.`,
    opt('draw', {
      label: `story:${key}`,
      phase: 'States',
      effort: 'high',
      schema: s({
        boards: list(BOARD), coverage: list(COVER), links: list(SCREEN_ACT), flows: list(FLOW),
        missingTokens: strs, handBuilt: strs, assumptions: ASSUMED, failing: strs,
      }, ['boards', 'coverage', 'links', 'flows', 'missingTokens', 'handBuilt', 'failing']),
    }),
  ).then((r) => (r ? { ...r, key, path: o.path, title: o.title, screens: o.screens, assumptions: [...(o.assumptions || []), ...(r.assumptions || [])] } : null))
}))).filter(Boolean)
if (!storyResults.length && !drawnScreens.length) throw new Error(`Neither a screen nor a story was drawn. The design doc ${DOC} holds the Components board only: set it aside with \`omni doc rename ${DOC} "Design, unfinished"\` and run again with pick.`)
if (!storyResults.length) log('No user story returned its boards; the design holds the screens\' normal states only')
const undrawn = outlines.filter((o) => !storyResults.some((r) => r.path === o.path))
if (undrawn.length && storyResults.length) log(`Stories outlined but not drawn: ${undrawn.map((m) => m.path).join(', ')}`)
missing.push(...undrawn.map((o) => ({ path: o.path })))

// ---- Wire ------------------------------------------------------------------
const allBoards = [...drawnScreens.map((d) => ({ id: d.id, name: d.name })), ...storyResults.flatMap((r) => r.boards)]
const wired = await wire(DOC, PAGE, storyResults.flatMap((r) => r.flows), storyResults.flatMap((r) => r.links), 'This is the first design: the page has no flows yet.', allBoards)

// ---- Check -----------------------------------------------------------------
phase('Check')
const handBuilt = [...drawnScreens.flatMap((d) => d.handBuilt), ...storyResults.flatMap((r) => r.handBuilt)]
const missingTokens = [...drawnScreens.flatMap((d) => d.missingTokens), ...storyResults.flatMap((r) => r.missingTokens)]
const completed = handBuilt.length || missingTokens.length ? await complete(DOC, DS, handBuilt, missingTokens, '') : null
const checked = (await check(DOC, storyResults.map((r) => ({
  key: r.key,
  path: r.path,
  title: r.title,
  boards: [...drawnScreens.filter((d) => r.screens.includes(d.key)).map((d) => ({ id: d.id, name: d.name })), ...r.boards],
  coverage: r.coverage,
})))).filter(Boolean)
const tokens = await tokenCheck(DOC, DS)
log(tokenLine(tokens))

// ---- Notes -----------------------------------------------------------------
phase('Notes')
const summary = {
  mode: 'first design',
  pick: A.pick,
  designSystem: { doc: DS, created: system.created, style: system.style || '', why: system.why || '' },
  screens: drawnScreens.map((d) => ({ id: d.id, name: d.name })),
  stories: storyResults.map((r) => ({ path: r.path, title: r.title, boards: r.boards })),
  notDrawn: [...screens.filter((sc) => !drawnScreens.some((d) => d.key === sc.key)).map((sc) => sc.name), ...missing.map((m) => m.path)],
  coverage: storyResults.flatMap((r) => r.coverage),
  mergedAcrossStories: merged ? merged.merges : [],
  assumptions: [...plan.assumptions, ...storyResults.flatMap((r) => r.assumptions || [])],
  review: checked,
  tokensAdded: completed ? completed.tokens : [],
  componentsAdded: completed ? completed.components : [],
  leftAsFrames: completed ? completed.left : handBuilt,
  wiring: wired || { flows: 0, dangling: ['the wiring step returned nothing'], unreachable: [] },
  tokenCheck: tokens,
  failing: [...system.failing, ...drawnScreens.flatMap((d) => d.failing), ...storyResults.flatMap((r) => r.failing)],
}
const notesDoc = await notes(DOC, summary)
return { feature: F, doc: DOC, link: link(DOC), notesDoc, ...summary }
