export const meta = {
  name: 'design-options',
  description: 'First half of designing a feature: research, then three grey layout options of its key screens side by side on one page of a design doc in OmniState, with a comparison note and a recommendation. Stops there; the human picks.',
  whenToUse: 'When a feature has a spec in OmniState and needs its screens laid out, or the layout rethought. Run with the Workflow tool as the workflow design-options with args {feature: "<name>"} (optional: mode ("new" | "redesign"), current (URL, screenshot folder or code paths of the screens today, for a redesign), screen ("1440x900"; default: the spec\'s main screen size), steer (what the human wants different from an earlier round), research ("reuse" (default) | "redo" | "skip"), models ({prepare, research, plan, draw, compare}; default: draw and prepare on sonnet, the rest on the session\'s model), model (one for all), skills (the skills folder; default ~/.claude/skills)), from a folder linked to the feature\'s OmniState project (omni link). Only the feature\'s Spec doc is required; it reads the behaviour tree, the Project spec, the design system and earlier research when they exist and says at its start what it found. Uses the skills design-research and ux-design. Writes to OmniState only: the html doc "Design research" and the design doc "Layout options" on the feature, and, when the feature has a behaviour tree, its assumptions as questions answered by default on the nodes. Work files go in a temp folder. A second run adds a new page of three options to the same doc. Next step: the human picks, then design-feature with args {feature, pick}.',
  phases: [
    { title: 'Prepare', detail: 'what exists: Spec, Project spec, behaviour tree, design system, earlier research, earlier options' },
    { title: 'Research', detail: 'design-research: comparable products, references, patterns, anti-patterns; reused when the doc exists' },
    { title: 'Plan', detail: 'ux-design steps 1 and 2: what the options must serve, three plans that differ, assumptions onto the tree' },
    { title: 'Options', detail: 'ux-design step 3: one agent per option draws its key screens in grey and looks at the images', model: 'sonnet' },
    { title: 'Compare', detail: 'ux-design step 4: the comparison note beside the options, a recommendation, one flow per option' },
  ],
}

const A = args || {}
if (!A.feature) throw new Error('args.feature is required: the name of the OmniState feature to design')
const F = A.feature
const MODE = A.mode === 'redesign' ? 'redesign' : 'new'
const SK = A.skills || '~/.claude/skills'
const UXD = `${SK}/ux-design`
const SLUG = F.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'feature'
const RUN = `design-${SLUG}`

// Drawing runs on a cheaper model than planning and comparing. Override per
// role with args.models, or with args.model for every role.
const MODELS = { prepare: 'sonnet', draw: 'sonnet', ...(A.models || {}) }
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

// ---- Prepare ---------------------------------------------------------------
phase('Prepare')
const prep = await agent(
  `Find what exists for the OmniState feature "${F}", so a design run can start. Read only; create nothing but the scratch folder. ${RULES('prepare')}

1. \`${omni('prepare')} link\` prints the project this folder is linked to: return its ID and name. If it prints no project, return linked false and stop.
2. Scratch folder: \`mktemp -d\`. Return its absolute path.
3. Spec: \`${omni('prepare')} doc list --feature "${F}" --pillar spec\`. Return the IDs of the feature's spec docs, and hasSpec true when at least one holds written content (\`${omni('prepare')} doc read <id>\`).
4. Project spec: \`${omni('prepare')} doc list --global --pillar spec\`. Return the ID of the app-level spec doc (the one titled "Project spec" when there are several), or "".
5. Behaviour tree: \`${omni('prepare')} --feature "${F}" behaviour list --json\`. Return nodeCount (0 when the feature has no tree or the command fails) and the user stories: the nodes at level 1, each with its path and title.
6. Design system: \`${omni('prepare')} doc list --global --pillar ui\`; a doc of kind design-system is the project's. Return its ID or "". When there is one, \`${omni('prepare')} tokens export\` and return as fontFamily the family its body text uses, or "".
7. Earlier work on the feature: \`${omni('prepare')} doc list --feature "${F}"\`. Return researchDoc, the ID of the html doc titled "Design research" (or ""), and optionsDoc, the ID of the design doc titled "Layout options" (or ""). For optionsDoc, \`${omni('prepare')} doc read <id>\` and return each <Page> with its id, its name and how many boards it holds.`,
  opt('prepare', {
    label: 'prepare',
    effort: 'low',
    schema: s({
      linked: bool, projectId: str, projectName: str, scratch: str,
      hasSpec: bool, specDocs: strs, projectSpecDoc: str,
      nodeCount: int, stories: { type: 'array', items: s({ path: str, title: str }) },
      designSystemDoc: str, fontFamily: str, researchDoc: str, optionsDoc: str,
      optionsPages: { type: 'array', items: s({ id: str, name: str, boards: int }) },
    }, ['linked', 'projectId', 'scratch', 'hasSpec', 'specDocs', 'nodeCount', 'stories']),
  }),
)
if (!prep || !prep.linked || !prep.projectId) throw new Error('This folder is not linked to an OmniState project, or omni is not signed in. Run `omni link <project>` (and `omni login` if needed), then run again.')
if (!prep.hasSpec) throw new Error(`The feature "${F}" has no Spec doc with content. Write its spec in OmniState first (omni-grill), then run again.`)

const S = prep.scratch
const TREE = prep.nodeCount > 0
const stories = prep.stories || []
const pages = prep.optionsPages || []
const usedPages = pages.filter((p) => p.boards > 0)
const ROUND = usedPages.length + 1
log(`Found: Spec (${prep.specDocs.length} doc${prep.specDocs.length === 1 ? '' : 's'}); Project spec: ${prep.projectSpecDoc ? 'yes' : 'none'}; behaviour tree: ${TREE ? `${stories.length} user stories, ${prep.nodeCount} nodes` : 'none, designing from the Spec alone'}; design system: ${prep.designSystemDoc ? 'yes' : 'none'}; research: ${prep.researchDoc ? 'exists' : 'none'}; earlier options: ${usedPages.length ? `${usedPages.length} round${usedPages.length === 1 ? '' : 's'}, this is round ${ROUND}` : 'none'}`)

const behaviour = (role) => (TREE
  ? `The behaviour: the feature's behaviour tree. \`${omni(role)} --feature "${F}" behaviour list\` prints every node; \`behaviour read <path>\` prints one with its rules and questions. Its ${stories.length} user stories: ${stories.map((x) => `${x.path} (${x.title})`).join('; ')}. The Spec doc${prep.specDocs.length === 1 ? '' : 's'} (${prep.specDocs.join(', ')}; \`doc read <id>\`) give${prep.specDocs.length === 1 ? 's' : ''} the background.`
  : `The behaviour: the feature has no behaviour tree, so the Spec doc${prep.specDocs.length === 1 ? '' : 's'} decide${prep.specDocs.length === 1 ? 's' : ''} it (${prep.specDocs.join(', ')}; read with \`${omni(role)} doc read <id>\`). Create no tree.`)
const today = (role) => (MODE !== 'redesign' ? ''
  : A.current ? ` The screens today: ${A.current}.`
    : role === 'research' ? ' The screens today were not named: find them in the project this folder holds (its routes and screen components), and say in the report what you read.'
      : ' The screens today: described in the research doc, section Current design.')
const inputs = (role) => [
  `Feature: "${F}". Scratch folder: ${S}. Mode: ${MODE}.${today(role)}`,
  behaviour(role),
  prep.projectSpecDoc ? `The Project spec (doc ${prep.projectSpecDoc}) says who the users are and what the product is.` : '',
  RULES(role),
].filter(Boolean).join(' ')

// ---- Research --------------------------------------------------------------
phase('Research')
let researchDoc = prep.researchDoc || ''
const RESEARCH_MODE = A.research || 'reuse'
if (RESEARCH_MODE === 'skip') {
  log('Research skipped (args.research)')
} else if (researchDoc && RESEARCH_MODE !== 'redo') {
  log(`Research reused from doc ${researchDoc}; pass research: "redo" to do it again`)
} else {
  const research = await agent(
    `Follow the skill at ${SK}/design-research/SKILL.md, all of it${TREE && stories.length <= 1 ? '; its Size line applies, this feature has one job' : ''}. ${inputs('research')}

Mobbin's tools may be connected (look for tools that search screens and flows); without them use web search. Save images in ${S}/images.

Destination: an OmniState html doc on the feature. ${researchDoc
    ? `Doc ${researchDoc} holds the earlier report: read it (\`${omni('research')} doc read ${researchDoc}\`) and, in ONE \`doc edit\` batch, replace its first top-level node with the new <section> and delete every other top-level node, each with the rev you read.`
    : `Create it: \`${omni('research')} doc create --feature "${F}" --kind html --pillar research --title "Design research"\`. Write the report as one <section> to ${S}/research.html and insert it: \`jq -Rs '[{op:"insert",parent:null,position:{after:null},content:.}]' ${S}/research.html | ${omni('research')} doc edit <doc id> --body-file -\`.`}

Return the doc's ID and the short summary the skill's hand-over describes.`,
    opt('research', { label: 'research', effort: 'medium', schema: s({ doc: str, summary: str }) }),
  )
  if (research && research.doc) researchDoc = research.doc
  else log('Research returned nothing; the options build on conventions')
}
const RESEARCH = researchDoc ? `Research: the html doc ${researchDoc} (\`doc read ${researchDoc}\`).` : 'There is no research: build on common conventions for this kind of product.'

// ---- Plan ------------------------------------------------------------------
phase('Plan')
const LETTERS = ['A', 'B', 'C']
const LENS = [MODE === 'redesign' ? 'Refine today' : 'Familiar', 'Task first', 'Bold']
const SCREEN = s({ name: str, layout: str, stories: strs })
const plan = await agent(
  `Do "Step 1: What the options must serve", "Step 2: Plan three options" and "Assumptions" of ${UXD}/SKILL.md, with the lenses and "What an option draws" in ${UXD}/references/wireframes.md. Plan only; draw nothing. ${inputs('plan')} ${RESEARCH}

The lenses: A ${LENS[0]}, B ${LENS[1]}, C ${LENS[2]}. Screen size: ${A.screen ? A.screen : 'choose it as wireframes.md says'}.
${usedPages.length ? `This is round ${ROUND}: the design doc ${prep.optionsDoc} already holds ${usedPages.length} earlier round${usedPages.length === 1 ? '' : 's'} of options (\`${omni('plan')} doc read ${prep.optionsDoc}\`), and the human may have commented on them (\`${omni('plan')} comment list --doc ${prep.optionsDoc} --json\`; a comment is open while waiting_on_you is false). The three new plans answer the open comments and differ from every earlier option.` : ''}
${A.steer ? `The human asks of this round: ${A.steer}` : ''}
${TREE
    ? `Assumptions go onto the tree as the skill says, each on the node it belongs to: \`${omni('plan')} --feature "${F}" behaviour ask <node path> "<question>" --answer "<the choice>"\`. Return each one you added.`
    : 'Return each assumption; the comparison note will list them.'}

Return the three plans in the order A, B, C (per screen: its name, its layout in a few lines, and the stories or jobs that happen there), the screen's width and height in points, the stories or jobs in one line each, the assumptions, and "same": everything the three options share and draw alike, written out so three agents who never see each other's boards draw it the same (the parts of the screen the feature leaves as they are, such as navigation and the page around it, with their items in order, and the sample data: names, rows, counts). "" when the options share nothing.`,
  opt('plan', {
    label: 'plan',
    effort: 'high',
    schema: s({
      width: int, height: int, jobs: strs, same: str,
      plans: { type: 'array', minItems: 3, maxItems: 3, items: s({ sentence: str, screens: { type: 'array', minItems: 1, maxItems: 6, items: SCREEN } }) },
      assumptions: { type: 'array', items: s({ node: str, question: str, answer: str }) },
    }),
  }),
)
if (!plan) throw new Error('The plan step returned nothing')
const W = plan.width
const H = plan.height
log(`Plan: ${plan.plans.map((p, i) => `${LETTERS[i]} ${p.screens.length} screens`).join(', ')} at ${W}×${H}; ${plan.assumptions.length} assumption${plan.assumptions.length === 1 ? '' : 's'}${TREE ? ' added to the tree' : ''}`)

const PAGE_NAME = ROUND === 1 ? 'Options' : `Options, round ${ROUND}`
const emptyPage = pages.find((p) => p.boards === 0)
const page = await agent(
  `Make the page that three layout options of the feature "${F}" will be drawn on. ${RULES('page')}
${prep.optionsDoc
    ? `The design doc is ${prep.optionsDoc}. ${emptyPage
      ? `Its page ${emptyPage.id} is empty: name it "${PAGE_NAME}", as ${UXD}/references/boards.md (Changing a board that exists) describes.`
      : `Add a page after its last one: read the doc for the last <Page> id, then send [{"op":"insert","parent":null,"position":{"after":"<last page id>"},"content":"<Page name=\\"${PAGE_NAME}\\"/>"}] to \`${omni('page')} design edit ${prep.optionsDoc} --body-file -\`.`}`
    : `Create the design doc: \`${omni('page')} doc create --feature "${F}" --kind design --pillar ui --title "Layout options"\`. It starts with one empty page: read it (\`${omni('page')} doc read <doc id>\`) and name that page "${PAGE_NAME}" with [{"op":"set","node":"<page id>","rev":1,"properties":{"name":"${PAGE_NAME}"}}] sent to \`${omni('page')} design edit <doc id> --body-file -\`.`}
Return the doc's ID and the page's ID.`,
  opt('prepare', { label: 'page', phase: 'Plan', effort: 'low', schema: s({ doc: str, page: str }) }),
)
if (!page || !page.doc || !page.page) throw new Error('The page for the options could not be made')
const DOC = page.doc

// ---- Options ---------------------------------------------------------------
phase('Options')
const NOTE_W = 560
const GAP = 240
const colX = (i) => NOTE_W + GAP + i * (W + GAP)
const FONT = prep.fontFamily ? `Typeface: "${prep.fontFamily}", the project's.` : 'Typeface: "Inter".'
const drawn = await parallel(LETTERS.map((L, i) => () => agent(
  `You draw option ${L} (${LENS[i]}) of three layout options, alone; the other two are not yours to read. Do "Step 3: Draw" of ${UXD}/SKILL.md for this one option: the format and the self-check in ${UXD}/references/wireframes.md, the rules in ${UXD}/references/rules.md, and ${UXD}/references/boards.md for writing boards and looking at their images. ${inputs(`option-${L.toLowerCase()}`)} ${RESEARCH}

Your plan: ${JSON.stringify(plan.plans[i])}
What the options must serve: ${plan.jobs.join(' | ')}
${plan.same ? `The same in all three options, so draw it exactly as written: ${plan.same}` : ''}
${plan.assumptions.length ? `Settled for all three options: ${plan.assumptions.map((a) => `${a.question} ${a.answer}`).join(' | ')}` : ''}

Draw into design doc ${DOC}, page ${page.page}, project ${prep.projectId}. Each board is ${W} wide and ${H} high, at x ${colX(i)}; the first at y 0, each next one ${H + 120} further down. Board names start with ${L}1, ${L}2, ... Above the column goes its label, as wireframes.md describes it: a board named "Option ${L}: ${LENS[i]}" at x ${colX(i)}, y -440, ${W} wide and 320 high. ${FONT} Your work files go in ${S}/option-${L.toLowerCase()}.

Look at the image of every board, also one made the same way as a board you have already looked at.

Return each board you drew (ID and name) in the order a person meets them, any screen of the plan you could not draw with the reason, and anything that still fails the self-check after three looks.`,
  opt('draw', {
    label: `option-${L}`,
    phase: 'Options',
    effort: 'medium',
    schema: s({ boards: { type: 'array', items: s({ id: str, name: str }) }, notDrawn: strs, failing: strs }),
  }),
)))
const alive = LETTERS.map((L, i) => ({ L, lens: LENS[i], sentence: plan.plans[i].sentence, x: colX(i), r: drawn[i] })).filter((o) => o.r && o.r.boards.length)
if (alive.length < 2) throw new Error(`Fewer than two options were drawn; the page ${page.page} of doc ${DOC} holds what there is`)
if (alive.length < 3) log(`Option ${LETTERS.filter((L) => !alive.some((o) => o.L === L)).join(', ')} was not drawn; comparing ${alive.map((o) => o.L).join(' and ')}`)

// ---- Compare ---------------------------------------------------------------
phase('Compare')
const compare = await agent(
  `Do "Step 4: Compare" of ${UXD}/SKILL.md for the layout options of the feature "${F}": the comparison note as ${UXD}/references/wireframes.md describes it, written and checked as ${UXD}/references/boards.md says, with the walkthrough of ${UXD}/references/judging.md and the rules in ${UXD}/references/rules.md. ${inputs('compare')} ${RESEARCH}

The options, on page ${page.page} of design doc ${DOC} (project ${prep.projectId}): ${JSON.stringify(alive.map((o) => ({ option: o.L, lens: o.lens, sentence: o.sentence, boards: o.r.boards, notDrawn: o.r.notDrawn, failing: o.r.failing })))}
What they must serve: ${plan.jobs.join(' | ')}
Assumptions made by the plan${TREE ? ' (already on the behaviour tree as questions answered by default)' : ' (list each in the note)'}: ${JSON.stringify(plan.assumptions)}

1. Look at every option's boards as images (work files in ${S}/compare).
2. Where a board shows something the behaviour or a settled assumption rules out (a button enabled that must be disabled, a missing line the plan promised), correct that layer on the board, as boards.md (Changing a board that exists) says, and look at it again. Change nothing about an option's layout or idea.
3. Write the board "Comparison" on the same page at x 0, y 0, ${NOTE_W} wide, height "hug", and under it the board "Assumptions" when the note calls for one. Look at their images too.
4. Set the page's flows: one per option drawn (${alive.length} here), named "Option <letter>: <lens>", starting at the option's first board.
5. \`${omni('compare')} design flows --doc ${DOC} --page ${page.page}\` must show no dangling target; fix one by pointing the interaction at the right board of the same option.
${usedPages.length ? `6. For each open comment on the doc (\`${omni('compare')} comment list --doc ${DOC} --json\`), reply with where this round answers it: \`${omni('compare')} comment reply <comment id> "<text>"\`.` : ''}

Return the recommended option's letter, the reason in one or two sentences, what is worth combining (or ""), the Comparison board's ID, each correction you made to an option board in one line, and anything that failed (a board that nothing leads to is not a failure when it is a note, a column label, or a state only a refusal reaches). For the link, \`${omni('compare')} config show | grep -E '^(server|workspace_id):'\` prints the server and the workspace ID (run it with that grep, so only those two lines are printed): return <server>/w/<workspace id>/projects/${prep.projectId}/docs/${DOC}.`,
  opt('compare', {
    label: 'compare',
    effort: 'high',
    schema: s({ recommended: str, reason: str, combine: str, noteBoard: str, link: str, corrections: strs, failures: strs }, ['recommended', 'reason', 'corrections', 'failures']),
  }),
)
if (!compare) throw new Error(`The comparison returned nothing; the options are on page ${page.page} of doc ${DOC}`)
if (compare.failures.length) log(`Open after the comparison: ${compare.failures.join('; ')}`)

return {
  feature: F,
  doc: DOC,
  page: page.page,
  round: ROUND,
  link: compare.link || '',
  options: alive.map((o) => ({ option: o.L, lens: o.lens, sentence: o.sentence, boards: o.r.boards.length, failing: o.r.failing })),
  recommended: compare.recommended,
  reason: compare.reason,
  combine: compare.combine || '',
  assumptions: plan.assumptions,
  assumptionsOn: TREE ? 'the behaviour tree, as questions answered by default' : 'the comparison note',
  researchDoc,
  corrections: compare.corrections,
  failures: compare.failures,
  next: `The human picks one option or combines parts of them, then: design-feature with args {feature: "${F}", pick: "<the pick in their words>"}`,
}
