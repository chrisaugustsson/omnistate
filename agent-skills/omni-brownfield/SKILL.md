---
name: omni-brownfield
description: Import an existing codebase into OmniState. Reads one or more repos, proposes the map (its parts and each part's features) for the human to confirm, then writes READMEs, setup guides, run books, architecture diagrams, how each feature works, the design system and each app feature's screens as a clickable prototype. Use when the user has code and no OmniState project for it yet, when an imported project's code has moved on, or says 'omni-brownfield'.
---

Turns code that already exists into an OmniState project: one project, its **parts** (each app or service, one folder in a repo; a monorepo gives several) and each part's **features** (what the part can do today). Everything else hangs off that **map**, so the human confirms the map before anything costly is written. After the import, new work on the codebase goes through the same steps as a new project's: tracks with their spec, behaviour, design, how, tasks and delivery.

Read the `omni` skill's Start first and follow it (`--as <run>/<role>` on every command).

## Out of bounds

Read source code, READMEs, docs, build files, Dockerfiles, compose files, Helm charts and CI config. Leave `.env` files, `secrets/` folders, keys, certificates, kubeconfigs and anything holding a password or token unopened, even when a README points at them; a setup guide names such a file and what it must hold, never its values. Seeds, fixtures and test data can hold customer data, device locations and personal data; describe their shape and leave the values out. The workflow passes these same bounds to every agent.

## Run

1. **Gather.** Ask the human, one question at a time, for what they have not given: the project's name and the repos it spans (absolute paths). `omni link` in each repo's folder shows whether it already belongs to a project; one that does is a [rerun](#rerun).
2. **Map.** Run the Workflow `omni-brownfield` with `args: {mode: "map", repos: [<paths>], omni: "<omni command>", run: "<run>"}`. `omni` is the command agents run, `omni` unless the human uses another server or binary (`OMNI_SERVER=<url> <path>/omni`). It returns the map: per repo its remote, its parts (name, kind `app` or `service`, folder, one line on what it is) and per part its features (name, one line on what it does today, whether it has screens, where it lives in the code), plus the shared packages that are no part.
3. **Confirm the map.** Show it as one table per repo: part, kind, folder, then its features with their lines; a large map goes in a file the human opens. Propose trimming the plumbing: a feature no user would name (health checks, deployment, migrations, metrics, a proxy that is not the part's purpose) moves to its part's `plumbing` list, covered in the part's README and run book, and a dev-only tool moves from the parts to its repo's `devTools`, covered in the project README. The human renames, merges, splits, drops or adds; apply each change and show the map again. Done when the human confirms it.
4. **Create.** In this order, each name exactly as the map writes it:
   - `omni project create "<name>"` (skip on a rerun), then in each repo's folder `omni link "<project>" --part "<part>=<folder>" ...` for that repo's parts, after the parts exist.
   - Per part `omni part create "<part>" --kind <kind> --description "<line>"`.
   - Per feature `omni feature create "<feature>" --part "<part>" --description "<line>"`.

   Done when `omni part list`, `omni feature list` and `omni part where <part>` (from any of the repos) show every part and feature of the map.
5. **Write.** Save the confirmed map as JSON in a scratch file and run the Workflow `omni-brownfield` with `args: {mode: "write", project: "<name>", mapFile: "<that file>", omni, run}` (`map: <the map>` inline works for a small one; `only: [<part>, ...]` limits a run to some parts, for a pilot; such a run skips the project docs and adds to the design system). Add `apps: {"<part>": "<url>"}` when the human has an app running and wants the boards checked against it. It writes, in parallel:
   - the project: its description (the Overview), a README, the Project spec, an architecture diagram of all parts, a setup guide and a run book;
   - each part: its description, a README with its repo link, a setup guide, a run book when the code shows how it runs, and an architecture diagram;
   - each feature: its description and a "How it works" doc with a diagram;
   - the design system, read from the UI code, and for each feature with screens a "Design" doc whose boards are its screens and states, wired into a prototype;
   - a last check that no doc holds a secret, a location or personal data, and that every board takes its values from tokens.
6. **Hand over.** Give the human the project's link and the workflow's report: what was written, the features it flagged, the screens it could not draw and why, and the tokens the design system still lacks.

## Rerun

When the project exists, the code has moved on since the import. The map run gets the project's current parts and features (`args.existing`, from `omni part list` and `omni feature list --part <part>`) and marks each part and feature as kept, new or gone. Confirm the map as above; create only what is new. A part or feature gone from the code stays in OmniState: list it for the human, who deletes it or keeps it. The write run gets `rerun: true`: each agent reads what OmniState holds, compares it with the code, and edits only the nodes and boards that no longer match, so the human's edits and comments stay.
