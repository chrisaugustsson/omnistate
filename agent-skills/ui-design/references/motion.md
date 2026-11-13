# Motion on boards

What `emil-design-eng` asks for, written as the interactions a board prototype can express. Read that skill for the reasons; this file is the translation.

## Which transition

| The moment | The interaction |
| --- | --- |
| A move people repeat all day: a tab, a sidebar item, a row to its detail at a desk, anything a key also does | `navigate`, transition `instant` |
| Going one level in on a phone (list to detail) | `navigate`, `push` `left`, 300 ms, the ease-out curve. The way back: `back` |
| A state appearing on the same screen (saved, an error under a field) | `navigate` to the state's board, `dissolve`, 150 ms; `smartAnimate`, 200 ms, when layers move between the two boards |
| A dialog | `openOverlay`, `position: "center"`, `background` the scrim, `dissolve`, 200 ms, the ease-out curve. Its Close and Cancel: `closeOverlay` |
| A sheet or a drawer | `openOverlay` at the edge it belongs to (`bottomCenter` for a sheet), `background` the scrim, `moveIn` from that edge's `direction`, 350 ms, the drawer curve |
| A menu, popover or select | `openOverlay` with `anchor: "triggerOutside"`, no `background`, `closeOnOutside`, `dissolve`, 150 ms |
| A tooltip | trigger `onHover`, `openOverlay` anchored to the trigger, `dissolve`, 125 ms, `delayMs` 400 |
| A toast | the state's board shows it; that board carries a trigger `afterDelay` (`ms: 4000`) with `navigate` to the board without it, `dissolve`, 200 ms |

## Curves and durations

- **Ease-out,** for anything entering or leaving: `{type: "customBezier", x1: 0.23, y1: 1, x2: 0.32, y2: 1}`.
- **Ease-in-out,** for something moving on screen: `{type: "customBezier", x1: 0.77, y1: 0, x2: 0.175, y2: 1}`.
- **Drawer:** `{type: "customBezier", x1: 0.32, y1: 0.72, x2: 0, y2: 1}`.
- Durations stay at 300 ms or under; sheets and drawers may take up to 500. Leaving is shorter than entering.
- Every transition answers "why does this move": it shows where something came from, that a state changed, or that the press was heard. A move with no such answer is `instant`.

The scrim is the one colour written as a literal on a finished board: an interaction's `background` takes `#rrggbbaa` only. Write the value of the design system's scrim token there.

## In the design system

The design system carries the same numbers as tokens in a `motion` group, so the code and the prototype agree: `motion.duration.pop` 150, `motion.duration.dialog` 200, `motion.duration.sheet` 350, `motion.duration.press` 160 (numbers, in ms) and `motion.ease.out`, `motion.ease.in-out`, `motion.ease.drawer` (strings, the curves as `cubic-bezier(...)`). An interaction's `durationMs` and `easing` take literal values: write the token's value.

## Beyond the prototype

Play mode cannot show these. List them under "Motion for the code" in the design notes, so they reach whoever builds the feature:

- **Press feedback:** a pressable element scales to 0.97 while pressed, 160 ms, ease-out.
- **Origin:** a menu or popover grows from its trigger, from scale 0.95 with opacity 0; a dialog grows from its centre.
- **Tooltips:** once one is open, the next opens at once, without delay or transition.
- **Interrupting:** a transition that is cut short reverses from where it is.
- **Reduced motion:** movement goes, fades stay.
- **Hover:** only on devices with a pointer that hovers.
