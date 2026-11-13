---
name: design-research
description: Design research for a product spec: how similar products solve the same jobs, screen and flow references from Mobbin (web search when Mobbin is not connected), patterns to borrow and anti-patterns, and for a redesign how the current screens work. Use before designing or redesigning screens, or when asked what comparable apps do.
---

# Design research

Gives the designers evidence instead of habit: what comparable products do for the same jobs, and what to avoid. The output feeds `ux-design` (its Familiar lens builds on it) and `ui-design`.

## Inputs

- **The spec** (required).
- **Mode:** *new* or *redesign*. A redesign also takes the current design: a URL, screenshots or the code of the screens.
- **Destination:** the caller's. With none named, the run folder is `research/<feature-slug>/` under the current directory. Images go in `images/` beside the report.
- **Size:** the counts in steps 2 and 3 are for a feature with several jobs. For a feature with one job, two comparable products and two references are enough.

## Step 1: The jobs

Read the spec and list its main jobs in one line each (what the user wants done, not which screen). These are the search terms for everything after. When the feature has a behaviour tree, its user stories are the jobs.

Done when every capability the spec describes belongs to a job.

## Step 2: Comparable products

Find three to six products that solve the same jobs for similar users: direct competitors first, then products from other domains that solve the same job well (a booking calendar, a kanban board, a design canvas). For each: what it is, which jobs it covers, and one or two things it does notably well or badly. Use web search; name sources.

Done when every main job has at least two products that solve it.

## Step 3: Screen and flow references

For each main job, collect references of real screens and flows:

- **Mobbin, when its MCP tools are connected** (search screens and flows by the job's words and pattern names: "empty state project list", "archive confirmation", "kanban board"). Keep the link and a screenshot when the tool returns an image or image URL.
- **Otherwise the web:** design system documentation (Material, Apple HIG, Carbon, Polaris, Atlassian, GitHub Primer), the products' own help pages and changelogs, and public galleries. Keep the link, and a screenshot when a browser tool can take one or the page offers an image URL; save images in `images/`.

Per reference record: the product, the job, the link, the image file if any, the pattern it shows in one line, and why it fits this spec. Five to ten references in all, at least one per main job. Images are for the team's internal use; record where each came from.

Done when every main job has at least one reference with a link, and each reference names its pattern and why it fits.

## Step 4: Patterns and anti-patterns

From steps 2 and 3, per job:

- **Patterns to borrow:** the recurring solution, the products that use it, and when it fits. Note where products disagree, since the disagreement is a design choice the layout options must make.
- **Anti-patterns:** two or three things comparable products do that hurt users, each with an example, and the rule from `ux-design`'s [rules.md](../ux-design/references/rules.md) it breaks, when one applies.

Done when every main job has at least one pattern and the report holds at least two anti-patterns.

## Step 5 (redesign only): The current design

How the screens work today: screens, data shown, every action, the states handled and the ones missing, permissions, and where the current design falls short of the spec. Cite the file or screen for each claim. When the caller provides a `ux-review` report (`review.html`) for these screens, take it as the starting point and add only what it lacks.

Done when every action of the current screens is listed and every claim has a file or screenshot behind it.

## Output: the report

Sections in this order: Jobs, Comparable products, References (grouped by job, with images), Patterns, Anti-patterns, Current design (redesign only), Sources.

- **As a file:** `research.html`, self-contained (inline CSS, no external resources; images by relative path from `images/`).
- **As an OmniState html doc,** when the caller names one: the same sections inside one `<section>` element, each image uploaded with `omni attach <file>` and shown as `<img src="attachment:<id>">`.

Hand over where the report is and a short summary: one line per main job naming its strongest pattern, and one line for the biggest anti-pattern.
