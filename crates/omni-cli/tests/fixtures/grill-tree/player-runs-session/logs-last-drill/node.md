---
status: done
same-as:
handoff:
---

# Logs the last drill

## Summary

Player logs the final drill.

## Situation

Player finishes the last drill of the session.

## Behavior

The summary screen shows every drill's score.

## Decisions

- Guest checkout? → Yes → No, login required
- Is the score shown at once? → Yes, on the summary. (default)

## Constraints

- The summary lists drills in the order played. (default)

## Children

- [Deletes on summary](./deletes-on-summary/) — Player deletes the session from the summary. (done)

## Open questions
