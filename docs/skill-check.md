# Skill check

The manual check of US-116: a fresh Claude Code session given only the skill creates a feature with a Spec doc and a design, and replies to a comment, with no other help.

Result: **PASS**, after one fix in the renderer (see Outcome)

## Setup

- Date: 2026-10-30
- Session: a separate `claude -p` process (Claude Code 2.1.312, model `claude-opus-5-5`, permission mode `auto`), started by the build agent from a work directory holding only the skill, linked to the project with `omni link`. Prompt 2 resumed the same session.
- Provisioned before the session started:
  - A stack of its own: `OMNI_PORT=8082 docker compose -p omnistate-skillcheck up --build --wait --detach proxy`, with a new account from `omni-api setup`.
  - `omni` on `PATH`, logged in to that stack.
  - The empty project Webshop, linked to the work directory with `omni link Webshop`.
  - The skill, copied unchanged from `skill/` to the work directory's `.claude/skills/omni/`, not to `~/.claude/skills/omni/`. A fresh session loads both the same way, and the init event listed `omni` among its skills.
- Not provisioned: an agent identity. The session had to learn `--as <run>/<role>` from the skill.
- Allowed tools, on top of what auto mode allows: `Skill`, `Read`, `Write`, `Edit`, `Bash(omni:*)`, `Bash(jq:*)`.
- The session also loaded the machine's global `~/.claude/CLAUDE.md` and global skills. Neither mentions OmniState or `omni`.

## Prompts

Prompt 1, the session's only input before the comment:

> The Webshop project needs a Saved carts feature: shoppers can save their cart and come back to it later. Shape it in OmniState: write its spec and a design of the screens, and tell me where to look.

Then the human step: the build agent, as the logged-in browser session, opened the design in the web app, selected the layer `Button: Save cart` (node `6a74ca1r`) and posted this through the comment composer:

> Save cart is easy to miss next to Checkout. Make this button stand out more, but keep Checkout the main action.

Prompt 2, in the same session:

> Read the comments.

## What the session ran

Prompt 1, in 23 turns and about 5 minutes, with no permission denials:

- `omni whoami --as saved-carts-1/shaper`, `omni config show`, `omni project list`
- `omni feature list`, `omni doc list`, `omni thread list`, `omni comment list`, `omni tokens export`, to see what the project already held
- `omni feature create` once
- `omni doc create` twice: the html doc Spec in `spec`, the design Saved carts screens in `ui`
- `omni doc edit` once for the spec, `omni design edit` four times for the boards (two refused, then corrected and resent)
- `omni doc show` and `omni doc read`, to read its work back
- `omni thread create` twice, for the two questions the human must decide

Prompt 2, in 5 turns and under a minute:

- `omni comment list`, then `omni doc read` to find the node
- `omni design edit` to change the button on boards 1, 3 and 7, and `omni doc edit` to bring the spec in line with it
- `omni comment reply` on the comment, then `omni comment list` again

The session also used the machine's global `design-taste-frontend` skill and Mobbin to choose a visual direction. That is design help, not OmniState help: every `omni` command came from the skill.

## Outcome

- The session set its own agent identity and created the feature Saved carts.
- The Spec has written content: 8 KB of HTML with why, what the shopper can do, rules and open questions.
- The design has 12 boards and 438 layers.
- The session gave the links to the feature, the Spec and the design, and put two open questions in threads.
- The comment landed on node `6a74ca1r`, by author `human`. The session replied as `saved-carts-1/shaper`, and `omni comment list` then showed it as `waiting on you`.
- In the web app, the Spec and the design both open, the reply shows beside the pin, and the Inbox lists the comment under Saved carts.

**The fix.** The check first failed here, after prompt 1: the design opened in the web app with no boards, only the error "the design system lists no modes". A project without tokens gave the renderer an empty token tree, and it refused that tree, although the API accepts it. Every new project starts that way, so the fix belongs in the renderer, not in a skill step that imports tokens first. `DesignSystem::from_json` now reads an empty token tree as a design system without tokens. The stack was rebuilt with its data kept; the session's design then opened unchanged, and the comment and prompt 2 ran after that.

## Friction

None of this stopped the session, and it needed no help. Each error named its own fix, except the first, which the session worked out from `omni: unknown option`.

- Keeping `--as ...` in a shell variable fails: zsh does not split `$A`, so `omni` saw one argument. The session then wrote `--as` as a flag and a separate value.
- `pad={{x: .., y: ..}}` is refused: `pad` takes `top`, `right`, `bottom` and `left`.
- `fill` is refused on `<Text>`: text takes `color`.
- One edit was refused because it inserted after a node that was not a child of the new parent.
