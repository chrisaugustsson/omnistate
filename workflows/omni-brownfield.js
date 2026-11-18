export const meta = {
  name: 'omni-brownfield',
  description: 'Imports an existing codebase into OmniState: maps one or more repos into parts and features, and after the human has confirmed the map writes READMEs, setup guides, run books, architecture diagrams, how each feature works, the design system and each app feature\'s screens as a clickable prototype.',
  whenToUse: 'Started by the omni-brownfield skill, twice: {mode: "map", repos: ["<absolute path>", ...], omni, run} returns the map to confirm with the human; {mode: "write", project: "<name>", map: <the confirmed map>, omni, run} writes everything once the project, parts and features exist. Optional: existing (on a rerun\'s map: the parts and features OmniState holds), rerun (write: edit only what no longer matches the code), apps ({"<part>": "<url>"}: a running app to compare boards with), only (part names to limit a write to), skills (the skills folder; default ~/.claude/skills), models ({map, docs, feature, system, boards, check}), model (one for all). omni is the command agents run (default "omni"; e.g. "OMNI_SERVER=<url> <path>/omni"). Reads code only; never opens .env files, secrets, keys or kubeconfigs, and keeps customer data, device locations and personal data out of what it writes. Writes to OmniState only; work files go in a temp folder.',
  phases: [
    { title: 'Scout', detail: 'map: one agent per repo finds its apps and services (the parts) and its shared packages' },
    { title: 'Features', detail: 'map: one agent per part reads its code and lists what it can do today' },
    { title: 'Project', detail: 'write: the project description, README, Project spec, architecture diagram, setup guide and run book' },
    { title: 'Parts', detail: 'write: per part its description, README with the repo link, setup guide, run book and architecture diagram' },
    { title: 'How it works', detail: 'write: per feature its description and a How it works doc with a diagram' },
    { title: 'System', detail: 'write: the design system from the UI code, one writer' },
    { title: 'Boards', detail: 'write: per feature with screens, its screens and states as boards wired into a prototype' },
    { title: 'Check', detail: 'write: tokens the boards lacked, the token check, flows, and a scan of every doc for secrets, locations and personal data' },
  ],
}

const A = args || {}
const MODE = A.mode
if (MODE !== 'map' && MODE !== 'write') throw new Error('args.mode is "map" or "write"')
const SK = A.skills || '~/.claude/skills'
const UI = `${SK}/ui-design`
const BOARDS = `${SK}/ux-design/references/boards.md`
const RUN = A.run || 'brownfield'
const OMNI = A.omni || 'omni'
const omni = (role) => `${OMNI} --as ${RUN}/${role}`

// Reading and drawing run on a cheaper model than mapping, the design system
// and the checks. Override per role with args.models, or args.model for all.
const MODELS = { feature: 'sonnet', boards: 'sonnet', ...(A.models || {}) }
const opt = (role, o) => {
  const model = A.model || MODELS[role]
  return model ? { ...o, model } : o
}

const s = (props, required) => ({ type: 'object', properties: props, required: required || Object.keys(props) })
const str = { type: 'string' }
const bool = { type: 'boolean' }
const strs = { type: 'array', items: str }
const list = (items) => ({ type: 'array', items })

// The bounds every agent works in (decision B9 of the Brownfield init spec).
const BOUNDS = `Bounds: read source code, READMEs, docs, build files, Dockerfiles, compose files, Helm charts and CI config. Leave .env files (any name starting with .env), secrets/ folders, keys, certificates, kubeconfigs and anything holding a password or token unopened, even when a README points at them; name such a file and what it must hold, never its values. Seeds, fixtures and test data can hold customer data, device locations and personal data: describe their shape and keep their values out of everything you return or write. A git remote URL loses any user:password@ part before you use it. Change nothing in any repository.`

const HTML = `An html doc: \`${OMNI} doc create <scope> --kind html --pillar <pillar> --title "<title>"\` prints its ID; write the whole body as one insert: put the HTML (one <section> holding everything) in a file in your scratch folder and run \`jq -Rs '[{op:"insert",parent:null,position:{after:null},content:.}]' <file> | <omni> doc edit <id> --body-file -\`. HTML, CSS and inline JavaScript work; nothing external (no CDN, fonts or images from the network). Diagrams are inline SVG: boxes for the parts or modules, arrows labelled with what goes over them (HTTP, a queue, a table), readable in light and dark (currentColor for lines and text). One sentence per line or list item, short sections with h2 headings, so a comment can point at one fact.`

const REWORK = A.rerun
  ? `This is a rerun: before writing a doc, look for it (\`${OMNI} doc list <scope>\`, same title); when it exists, read it, compare it with the code, and replace or set only the nodes that no longer match (the omni html ops replace/set with the node's rev), so the human's edits and comments stay. Create it only when it is missing.`
  : ''

if (MODE === 'map') {
  const repos = Array.isArray(A.repos) ? A.repos : []
  if (!repos.length) throw new Error('args.repos lists the repos to map, as absolute paths')
  const existing = A.existing || null

  const PART = s({ name: str, kind: { type: 'string', enum: ['app', 'service'] }, folder: str, line: str, stack: str }, ['name', 'kind', 'folder', 'line'])
  const FEATURE = s({ name: str, line: str, screens: bool, where: strs, status: { type: 'string', enum: ['kept', 'new', 'gone'] } }, ['name', 'line', 'screens', 'where'])

  phase('Scout')
  const mapped = await pipeline(
    repos,
    (repo) => agent(
      `Map the repo at ${repo} for an import into OmniState. ${BOUNDS}

A part is one app or service the repo builds and runs on its own: a web or mobile app (kind app), or a server, worker or CLI (kind service). A monorepo gives several parts, one per folder. A shared package (a UI kit, an API client, a library other folders import) is no part: list it under shared, with what reads it. Infrastructure (Helm, Terraform, CI) is no part either; note where it lives in notes.

1. \`git -C ${repo} remote get-url origin\` (with any user:password@ part removed) and the default branch.
2. Read the README and docs at the root, the workspace files (package.json workspaces, pnpm-workspace.yaml, turbo.json, nx.json, Cargo.toml, go.work, docker-compose files) and the folder layout, then each candidate folder's own README and build file.
3. Name each part as the people working on it would: the folder's name unless its README or package says otherwise.

Return every part with its folder relative to the repo root, a line on what it is for its users, and its stack in a few words.${existing ? ` OmniState already holds these parts for this project: ${JSON.stringify(existing.parts || [])}. Use the same name for a part that is the same thing.` : ''}`,
      opt('map', {
        label: `scout ${repo.split('/').pop()}`,
        phase: 'Scout',
        schema: s({ repo: str, remote: str, branch: str, parts: list(PART), shared: list(s({ folder: str, line: str, readBy: strs })), notes: str }, ['repo', 'remote', 'parts', 'shared']),
      }),
    ),
    (scout, repo) => parallel((scout?.parts || []).map((part) => () =>
      agent(
        `List the features of the part "${part.name}" (${part.kind}: ${part.line}) at ${repo}/${part.folder}, for an import into OmniState. ${BOUNDS}

A feature is something the part can do today that a user or another part would name: "Device list", "Alarm rules", "Export readings", "Readings ingest". Not a technical layer (routing, logging, auth middleware) unless the part exists for it. Aim for what a product owner would list on one page: usually 4 to 20 per part. Read the code to find them: for an app its routes, pages and navigation; for a service its endpoints, handlers, consumers and jobs. Read until every route and endpoint belongs to a feature or is plainly plumbing.

Per feature return its name, one line on what it does today, written for the people who use it, whether it has screens (screens true only for an app with pages for it), and where it lives (folders or files, relative to the repo). Also return the part's description: two or three sentences on what it is, who uses it and what it talks to.${existing ? `
OmniState already holds these features for this part: ${JSON.stringify(((existing.features || {})[part.name]) || [])}. Mark each feature kept (same thing, same name), new, or gone (in OmniState, no longer in the code; return those too, with their name).` : ''}`,
        opt('feature', {
          label: `features ${part.name}`,
          phase: 'Features',
          schema: s({ description: str, features: list(FEATURE) }),
        }),
      ).then((read) => ({ ...part, description: read?.description || part.line, features: read?.features || [] })),
    )).then((parts) => ({ repo, remote: scout?.remote || '', branch: scout?.branch || '', shared: scout?.shared || [], notes: scout?.notes || '', parts: parts.filter(Boolean) })),
  )
  return { repos: mapped.filter(Boolean) }
}

// ---- write ------------------------------------------------------------------
const P = A.project
if (!P) throw new Error('args.project names the OmniState project')
// The confirmed map, given inline (args.map) or as a JSON file (args.mapFile) that one agent reads.
const FEATURE_IN = s({ name: str, line: str, screens: bool, where: strs, status: str }, ['name', 'line', 'screens'])
const PART_IN = s({ name: str, kind: str, folder: str, line: str, stack: str, description: str, features: list(FEATURE_IN), plumbing: list(s({ name: str, line: str, where: strs }, ['name', 'line'])) }, ['name', 'kind', 'folder', 'line', 'features'])
const REPO_IN = s({ repo: str, remote: str, branch: str, notes: str, shared: list(s({ folder: str, line: str }, ['folder', 'line'])), devTools: list(s({ name: str, folder: str, line: str }, ['name', 'folder', 'line'])), parts: list(PART_IN) }, ['repo', 'remote', 'parts'])
const MAP = A.map && Array.isArray(A.map.repos)
  ? A.map
  : A.mapFile
    ? await agent(`Read the JSON file ${A.mapFile} (\`cat\` it) and return its content exactly as it is, every repo, part, feature and field, changing nothing.`, { label: 'read map', phase: 'Project', effort: 'low', model: 'sonnet', schema: s({ repos: list(REPO_IN) }) })
    : null
if (!MAP || !Array.isArray(MAP.repos)) throw new Error('args.map (or args.mapFile) is the confirmed map returned by the map run')
const ONLY = Array.isArray(A.only) && A.only.length ? new Set(A.only) : null
const PARTS = MAP.repos.flatMap((r) => r.parts.map((p) => ({ ...p, repo: r.repo, remote: r.remote, branch: r.branch })))
  .filter((p) => !ONLY || ONLY.has(p.name))
const MAP_TEXT = JSON.stringify(MAP.repos.map((r) => ({ repo: r.repo, remote: r.remote, shared: r.shared, devTools: r.devTools || [], notes: r.notes, parts: r.parts.map((p) => ({ name: p.name, kind: p.kind, folder: p.folder, line: p.line, stack: p.stack, features: p.features.filter((f) => f.status !== 'gone').map((f) => f.name) })) })))
const SCOPE = `Every omni command runs with \`--project "${P}"\` and starts \`<omni>\`, which is \`${OMNI} --as ${RUN}/<your role>\`. Make your own scratch folder with mktemp -d.`
const patch = (path) => `\`${OMNI} config show | grep -E '^workspace_id:'\` gives the workspace ID; IDs of the project, part and feature come from \`${OMNI} project list\`, \`${OMNI} part show "<part>" --project "${P}"\` and \`${OMNI} feature show "<feature>" --part "<part>" --project "${P}"\`. Set a description with \`jq -n --arg d "<text>" '{description:$d}' | <omni> api PATCH /api/v1/w/<workspace>/projects/<project>${path} --body-file -\`.`

phase('Project')
const projectDocs = agent(
  `Write the project-level docs of the OmniState project "${P}", imported from existing code. The confirmed map: ${MAP_TEXT}. ${BOUNDS} ${SCOPE} Your role is "project". ${REWORK}

Read each repo's README, docs folder, Makefile or task runner, compose files, CI config and deployment config (Helm, Terraform, GitOps) before writing.

1. The project's description is its Overview page: one paragraph on what the product is and who uses it, then a line per repo with its remote URL. ${patch('')}
2. "README" (html, --global, pillar spec): what the product is, its parts and what each is for (one line each), the repos with their links, the shared packages and dev tools the map lists with a line each, where to start reading.
3. "Project spec" (html, --global, pillar spec): exactly six h2 parts, in this order, one sentence per line or list item: Idea; Who it is for; Rules (what holds across the code: languages, auth, tenancy, i18n, conventions you see enforced); Stack (a line per choice, as the code shows it); Features (the parts, each with its features by name, as an ordered list per part); Not building (what the code plainly leaves to others, or "Not settled yet.").
4. "Architecture" (html, --global, pillar architecture): an SVG diagram of every part and what lies outside them (databases, queues, identity provider, third-party APIs), arrows labelled with what goes over them; then a section per part with a line on its role and what it talks to.
5. "Setup guide" (html, --global, pillar spec): what a developer installs, how each repo is checked out and started locally, in order, with the commands from the repos; the config files each needs, by name and purpose, never their values.
6. "Run book" (html, --global, pillar spec): how the product is built, deployed and observed, as the CI and deploy config show it: environments, pipelines, where logs and metrics go, health checks, and what to do for the failures the code anticipates. Write only what the config shows; say plainly where it shows nothing.

${HTML}

Return the doc IDs and a line on anything you could not find in the code.`,
  opt('docs', { label: 'project docs', phase: 'Project', schema: s({ docs: list(s({ title: str, id: str })), gaps: strs }) }),
)

const UI_PARTS = PARTS.filter((p) => p.kind === 'app' && p.features.some((f) => f.screens && f.status !== 'gone'))

// The design system: one writer, since every token and component write sends the whole tree.
const system = UI_PARTS.length
  ? agent(
    `Build the design system of the OmniState project "${P}" from its UI code. ${BOUNDS} ${SCOPE} Your role is "system".

The apps with screens: ${JSON.stringify(UI_PARTS.map((p) => ({ part: p.name, repo: p.repo, folder: p.folder })))}. Shared packages the map found (a UI kit among them feeds this): ${JSON.stringify(MAP.repos.map((r) => ({ repo: r.repo, shared: r.shared })))}.

Read ${UI}/SKILL.md, section "Step 1: The design system", and follow it with the product's code as the source: the style values (CSS custom properties, Tailwind or theme config, token files, the UI kit's components) are what the product looks like. When the project already has a design system (\`${OMNI} doc list --global --pillar ui --project "${P}"\`, kind design-system), compare it role by role as that section says and change it only where the code differs${A.rerun ? '' : ' (on a first import there is none: create it)'}. Where two apps style the same role differently, keep the shared value in the common token and give the app that differs its own token under a group named after the part, with a $description saying so.

Define as components what the screens repeat (button, field, nav item, list row, table, card, pill, dialog shell, the app shells' sidebar and top bar), built on tokens only. Done when every role the code styles has a token, every repeated element has a component, and a board named Components shows each one as an image that looks right.

Return the design-system doc ID, the token groups, the component names, and a line per role you could not read from the code.`,
    opt('system', { label: 'design system', phase: 'System', schema: s({ doc: str, groups: strs, components: strs, gaps: strs }) }),
  )
  : Promise.resolve(null)

const FEATURE_DONE = s({ feature: str, doc: str, gaps: strs })
const BOARDS_DONE = s({ feature: str, doc: str, boards: list(s({ name: str, id: str })), flows: strs, missingTokens: list(s({ path: str, value: str, type: str, why: str })), notDrawn: strs })

const partWork = pipeline(
  PARTS,
  (part) => agent(
    `Write the docs of the part "${part.name}" (${part.kind}) of the OmniState project "${P}". It lives in ${part.repo}/${part.folder}; the repo's remote is ${part.remote || 'unknown'}${part.branch ? `, branch ${part.branch}` : ''}. Its features: ${JSON.stringify(part.features.filter((f) => f.status !== 'gone').map((f) => ({ name: f.name, line: f.line, where: f.where })))}.${(part.plumbing || []).length ? ` Plumbing that is no feature of its own, to cover in its README and Run book: ${JSON.stringify(part.plumbing)}.` : ''} ${BOUNDS} ${SCOPE} Your role is "part". Docs on this part take \`--part "${part.name}"\`. ${REWORK}

Read the part's README, build file, Dockerfile, its entry point and the config it reads (by name), and how it is deployed (CI jobs, Helm values, compose services that name it).

1. Its description is its Overview: two or three sentences on what it is, who uses it and what it talks to, then "Code: <remote URL>/tree/<branch>/${part.folder}" when the remote is on GitHub or GitLab, else the repo path and folder. ${patch('/parts/<part>')}
2. "README" (html, pillar spec): what it is, its features with a line each, the folder layout that matters, the main dependencies, and the link to its code.
3. "Setup guide" (html, pillar spec): how to run it locally and test it, with the commands from its build file and the README; the config and environment it needs by name and purpose, never values.
4. "Run book" (html, pillar spec), only when the code shows how it runs: how it is built and deployed, its health checks, logs and metrics, and what to do for the failures its code handles (retries, dead letters, timeouts). Skip it when nothing shows how it runs.
5. "Architecture" (html, pillar architecture): an SVG diagram of the part's inside (its modules or layers, its data stores) and what it talks to outside (other parts by name, external services), arrows labelled; then a line per module.

${HTML}

Return the doc IDs and a line per gap.`,
    opt('docs', { label: `part ${part.name}`, phase: 'Parts', schema: s({ part: str, docs: list(s({ title: str, id: str })), gaps: strs }) }),
  ),
  (partDone, part) => parallel(part.features.filter((f) => f.status !== 'gone').map((feature) => () =>
    agent(
      `Write how the feature "${feature.name}" of the part "${part.name}" works, in the OmniState project "${P}". What it does: ${feature.line}. Where it lives: ${(feature.where || []).join(', ') || `${part.folder}, find it`} in ${part.repo}. ${BOUNDS} ${SCOPE} Your role is "feature". Docs on this feature take \`--feature "${feature.name}" --part "${part.name}"\`. ${REWORK}

Read its code end to end: for an app its pages, components, state and the API calls they make; for a service its endpoints, handlers, jobs, the tables it reads and writes and the messages it sends.

1. Its description: two sentences on what it does for its users today. ${patch('/parts/<part>/features/<feature>')}
2. "How it works" (html, pillar architecture): an SVG diagram of the path through the code (screen or entry point, the modules it passes, the API calls, the tables or queues, other parts), arrows labelled; then sections: What it does (a line per capability), Where it lives (files and folders), Data (tables, fields that matter, messages, by name), Rules (the checks, limits and permissions the code enforces), Edges (what it does on errors, empty states and timeouts, as the code handles them).

${HTML}

Return the doc ID and a line per gap.`,
      opt('feature', { label: `how ${feature.name}`, phase: 'How it works', schema: FEATURE_DONE }),
    ),
  )).then((features) => ({ ...(partDone || { part: part.name }), features: features.filter(Boolean) })),
)

const boardWork = system.then((sys) => parallel(UI_PARTS.flatMap((part) => part.features.filter((f) => f.screens && f.status !== 'gone').map((feature) => () =>
  agent(
    `Draw the screens of the feature "${feature.name}" of the app "${part.name}" in the OmniState project "${P}" as design boards, as the code builds them today, wired into a clickable prototype. What it does: ${feature.line}. Where it lives: ${(feature.where || []).join(', ') || part.folder} in ${part.repo}. ${BOUNDS} ${SCOPE} Your role is "boards".

The design system is the project's (doc ${sys?.doc || 'see doc list --global --pillar ui'}; \`${OMNI} tokens export --project "${P}"\` and \`${OMNI} doc read <design system doc>\` print its tokens and components). You write neither tokens nor components: another agent may be drawing beside you. Where a value is missing, use the closest token and report the value in missingTokens.

1. Read the feature's pages and components: every route, its layout, the components it renders, its real copy (the i18n strings in the main language), its states (loading, empty, error, a dialog, a form with a refused value) and the links and buttons that lead to another screen.
2. Its design doc: "Design" on the feature (\`${OMNI} doc create --feature "${feature.name}" --part "${part.name}" --kind design --pillar ui --title "Design" --project "${P}"\`), or the existing one${A.rerun ? ', changed in place where the code changed' : ''}.
3. Plan the boards as ${UI}/SKILL.md "Step 2: Plan the boards" says, with the code as the behaviour: one board per screen and per state the code renders. Then draw them as "Step 3: Draw" says (tokens only, instances of the system's components, the app's shell on every screen), following ${BOARDS} for writing boards and looking at their images. Data on a board is realistic and made up: never copy customer names, addresses or device locations from fixtures.
4. Wire them as "Step 4: Wire the prototype" says: each control that leads to another screen of this feature carries its interaction; one flow per path a user takes through the feature. A control that leads to another feature's screen leads nowhere here: list it in flows as "to <feature>".${A.apps && A.apps[part.name] ? `
5. The app runs at ${A.apps[part.name]}. Open each screen with the browser tools without signing in or typing any credential; where it shows, compare it with your board and fix the board where they differ. A screen behind a sign-in is compared from the code only.` : ''}

Done when every route and rendered state of the feature has a board, \`${OMNI} design flows --doc <doc>\` shows no dangling target, and each board's image matches what the code renders.

Return the doc, its boards, the flows, the tokens you lacked and what you could not draw and why.`,
    opt('boards', { label: `boards ${feature.name}`, phase: 'Boards', schema: BOARDS_DONE }),
  ),
))))

const [project, parts, boards, sys] = await Promise.all([projectDocs, partWork, boardWork, system])
const drawn = (boards || []).filter(Boolean)
const missing = drawn.flatMap((b) => b.missingTokens || [])

phase('Check')
const tokenFix = missing.length
  ? await agent(
    `Add the tokens the board agents lacked to the design system of the OmniState project "${P}", and put them on the layers that used the closest token instead. ${SCOPE} Your role is "system". The lacks, per feature: ${JSON.stringify(drawn.map((b) => ({ feature: b.feature, doc: b.doc, missing: b.missingTokens })))}. Merge lacks that are the same role into one token. Follow "Tokens only" in ${UI}/SKILL.md for how to add a token. Return the tokens added and the boards changed.`,
    opt('system', { label: 'missing tokens', phase: 'Check', schema: s({ added: strs, boards: strs }) }),
  )
  : null

const check = await agent(
  `Check what the import wrote into the OmniState project "${P}". ${SCOPE} Your role is "check".

1. Every doc: \`${OMNI} doc list --project "${P}"\` with --global, then per part (--part) and per feature (--feature --part), and read each html doc. Look for anything out of bounds: a secret (a key, token, password, connection string with credentials, private URL with a token), a device location (coordinates, an address, a site description tied to a device), or personal data (a real person's name, email or phone number taken from the code's data; a maintainer named in a README is fine). Remove each one in place with a replace op and list where it was, never its value.
2. Every design doc of kind design: run the token check at the end of ${UI}/SKILL.md "Step 5: Check" over the doc and the design system, and \`${OMNI} design flows --doc <doc>\`. Fix literals with the closest token and dangling targets by pointing them at the right board.
3. Return what you fixed, and per part a line on what is missing (a part without a README or Architecture, a feature without a How it works doc, an app feature without boards).`,
  opt('check', { label: 'check', phase: 'Check', schema: s({ removed: strs, fixed: strs, missing: strs }) }),
)

return {
  project,
  system: sys,
  parts: (parts || []).filter(Boolean),
  boards: drawn.map((b) => ({ feature: b.feature, doc: b.doc, boards: (b.boards || []).length, notDrawn: b.notDrawn })),
  tokens: tokenFix,
  check,
}
