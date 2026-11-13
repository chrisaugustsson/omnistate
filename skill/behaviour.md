# Behaviour tree

A feature has one behaviour tree: what the product does in each situation, case by case, written from the user's side of the screen. It is a tree of records, not a doc. You read and write it with `omni behaviour ...` (`behavior` is the same word), and the human reads it on the feature's Behaviour screen, where they answer questions, change answers and add remarks. The `omni-behaviour` skill fills a tree; this file is for any agent that reads or writes one.

## The model

- **Three levels.** The root is the feature (`.`). Its children are the user stories, each titled with a story sentence: "As a <role>, I want to <flow> so that <purpose>." Below a story come situations, as deep as they go. A branch ends in a leaf: an outcome the user sees.
- **A node** holds a title (the condition that leads here), a one-sentence summary, the situation (where the user is and what they just did), the behaviour (what the product does and what the user sees) and a status: `open` (not settled), `done`, or `out-of-scope` (excluded, kept visible). You name a node by its ID or its path: the slugs from the story down, joined by `/`.
- **Pointers.** `same-as` says the situation is described at another node; `handoff` says the user carries on in another story. A node has one pointer at most and then no children. Pointers stay inside the feature; something outside it is named in plain text.
- **Questions.** Each question on a node is a record, open or answered. An answer is a person's, or an agent's own (a default), and a changed answer keeps the old ones. A node with an open question cannot be `done`. An open question a person asked is a remark that waits for an agent.
- **Rules.** A rule is something a node imposes on other nodes, with a reach: the node and below, its story, or the whole feature. A rule that no longer holds is withdrawn, never deleted. Rules for the whole product live in the Project spec.
- **Needs another pass.** The server marks a node when a person writes on it, and everything below a changed answer. An agent clears the mark with `omni behaviour edit <node> --grilled` once it has brought the node in line.
- Nothing is deleted.

## Reading

- `omni behaviour read <node>` prints what one node needs: the nodes above it in full, the node, the rules that reach it, its children and its siblings. Read a branch this way instead of the whole tree.
- `omni behaviour list [<node>]` prints one summary line per node; `--needs-pass` only the marked ones.
- `omni behaviour questions [<node>] --open` lists what nobody has answered, `--default` what an agent answered on its own. `omni behaviour rules [<node>]` lists the rules in force.
- `--json` carries what the text leaves out: per node `open_questions`, `default_answers` and `needs_pass`; per question who asked it (`asked_by.kind`, `human` or `agent`) and who gave its answer (`answered_by`).

When you design or cut stories from a tree: a rule that reaches a node binds it, an answered question is settled, and an open question is not decided. Say that it is open instead of building on a guess.

## Writing

- Every command takes the feature from `--feature <name|id>` or `OMNI_FEATURE`.
- `omni behaviour add` sets the title, slug, summary and pointer; the node starts `open`. Its situation, behaviour and status come from `omni behaviour edit`.
- Write behaviour from the user's side: "the person presses Save and nothing happens for ten seconds", not "the rename request times out".
- Your answer is a default. Pass `--by human` only when a person told you the answer and you are writing it down.
- A person's answer is theirs: `--change` on it is refused (`answered_by_human`). Ask a new question on the node instead.
- `omni behaviour ask` on a done node sets it to `open`; with `--answer` the question is recorded settled and the status stays.
- A question you leave open for a person takes your recommendation with `--recommend "<the answer as it should stand>"`. The screen shows it as a proposed answer the person can accept with one press, so keep it out of the question's text.
- `omni behaviour answer --change` marks everything below the node for another pass.
- A write that exits 3 lost to a newer version: read again and resend.
