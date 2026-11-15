---
name: omni-init
description: Start a new project in OmniState. Grills the idea, writes the Project spec and creates the first features. Use when the user has an idea for a new product and no OmniState project for it yet, or says 'omni-init'.
---

Interview the user about a new product the way `~/.claude/skills/grilling/SKILL.md` does, and keep the answers in OmniState. Read that skill first and follow it: design tree, rounds, numbered questions with a recommended answer each. This skill adds what the init asks and where the answers go.

The init ends in OmniState: a project, its Project spec and its first features. The folder on disk stays as it is. Setting up the code is the first task the tasks step cuts for the first feature.

## Start

1. Follow steps 1 and 2 of the `omni` skill's Start (`omni whoami --as <run>/<role>`, `--as` on every command).
2. `omni link` shows whether the folder already belongs to a project.
   - No link: ask the human for the project's name when they have not given it, then `omni project create "<name>"` and `omni link "<name>"`.
   - A link to a project with a Project spec (`omni doc list --global --pillar spec`): this is a rerun. Go to [Run again](#run-again).
   - A link to a project without one: keep the project and continue.
3. Create the Project spec: `omni doc create --global --kind html --pillar spec --title "Project spec"`, and write it with all six headings, each part holding the one line `Not settled yet.` (see the `omni` skill's `html-docs.md` for how to write nodes).

Done when `omni link` names the project and the Project spec holds its six headings.

## The Project spec

Every agent in every feature of this project reads this doc before it works, so each line costs on every run. Six parts, in this order, one sentence per line, one line per list item:

- `Idea`: one paragraph on what the product is.
- `Who it is for`: the kinds of users, a line each.
- `Rules`: what holds in every feature, a line per rule.
- `Stack`: the overall technology, a line per choice.
- `Features`: an ordered list of the first features, in the order to build them, each with its name and one line on what it does.
- `Not building`: what the product leaves out, for now or for good.

A finished one, for a booking app for tyre workshops:

```html
<section>
<h1>Project spec</h1>
<h2>Idea</h2>
<p>Car owners book tyre changes online, and workshops see their day in one calendar.</p>
<h2>Who it is for</h2>
<ul><li>Car owner: books and changes a time.</li><li>Workshop staff: run the day from the calendar.</li></ul>
<h2>Rules</h2>
<ul><li>Everything is in Swedish and English.</li><li>Prices are shown with VAT.</li></ul>
<h2>Stack</h2>
<ul><li>Web app in React.</li><li>Server in Rust.</li><li>Postgres.</li><li>Hosting: open.</li></ul>
<h2>Features</h2>
<ol><li>Book a time: a car owner picks a workshop and a free time.</li><li>Workshop calendar: staff see and move the day's bookings.</li><li>Reminders: the car owner gets a message the day before.</li></ol>
<h2>Not building</h2>
<ul><li>Payments.</li><li>Tyre storage.</li></ul>
</section>
```

The doc holds results only. The questions and the reasons stay in the chat.

## The grill

The design tree of an init is the six parts: the idea first, then who it is for, then the features, rules, stack and what is left out, which all hang off the first two.

Hold every question to one test: **does every feature need the answer?**

- Yes: ask it, with an example in this product's own terms, since questions at this level are abstract ("Does a workshop see other workshops' bookings?").
- It concerns what one feature does in one situation: it belongs to that feature's own grill. Say so and move on; the feature keeps its one line.

Per part:

- **Features.** Settle which ones come first and their order. Propose the features the idea implies and the human did not name. A feature the human wants later goes under `Not building`.
- **Rules.** A look or brand is asked by the first feature's UI step; when the human names one unasked, it is a rule like any other.
- **Stack.** Ask what the human already knows they want to use. Recommend a choice only where the idea or the users force it (staff who work on a phone, a calendar that updates live); otherwise recommend `open`. A choice the human has no opinion on is written as `<what>: open`, and the setup task settles it. Folder layout, libraries and commands belong to the setup task.

After every round, write the Project spec: `set` or `replace` the lines that changed and `insert` new ones, so the human's comments stay on nodes that still exist. Give the human the doc's link after each round (the `omni` skill's Recipes say how it is built).

Run rounds until the frontier is empty and no part still says `Not settled yet.` Then give a scope summary of the six parts. The human confirms or corrects it; correct the doc and ask again until confirmed.

## Create the features

After the confirmation, and in the list's order: `omni feature create "<name>" --description "<the feature's line>"` for each feature, with the name exactly as the Project spec writes it. An agent reaches a feature from the Project spec by that name.

Done when `omni feature list` shows every feature of the list under the same name.

## Hand off

Give the human the link to the Project spec and name the next step: `omni-grill` on the first feature of the list.

Adding a feature later is `omni-grill`'s job, as is a new rule that comes up while a feature is grilled.

## Run again

For when the idea itself has changed, such as a new kind of user.

1. Read the Project spec (`omni doc read <doc>`) and `omni feature list`.
2. Ask the human what changed, and grill only the parts that hang off it.
3. Edit the lines that change. A feature that is new gets created as above; one the product drops moves to `Not building` and stays in OmniState until the human asks for its removal.
4. End with a summary of what changed, confirmed by the human.
