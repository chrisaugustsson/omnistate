# Walkthrough and severity

How to judge a design with no users in the loop. `ux-design` uses it to compare its options, `ui-design` to check the finished boards, and `ux-review` to review a design that exists.

## Walkthrough

Take one task (a user story, or a top task) and go through it step by step on the design. At each step ask:

1. Will the user try to do this?
2. Will they notice the control?
3. Will they connect the control to their goal?
4. Will they see that they made progress?

A *no* fails the step. Log the step, the question and why.

## Findings

Each finding names the element (the board and the layer), the task, the rule ID from [rules.md](rules.md) and the consequence for the user. Findings about aesthetics or overall purpose are marked *low confidence* and count at half their severity.

## Severity

Rate in a separate round, after all findings are in.

- **4:** blocks a task or loses data.
- **3:** seriously slows or misleads.
- **2:** a real but minor problem.
- **1:** cosmetic.

Anything that rates 0 is not a finding.
