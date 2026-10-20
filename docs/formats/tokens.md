# Design tokens

A project's design system doc holds its design tokens as one JSON tree in the
W3C Design Tokens (DTCG) format, with one value per mode (Q27). The checks in
`omni_core::design::tokens` (`TokenTree::from_json`) decide what a tree may
say; the API runs them on every import, so a tree that is stored always passes.
A project has at most one design system doc, started in a change set like any
doc, and `omni tokens import` replaces its whole token tree in one batch.

```json
{
  "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
  "color": {
    "$type": "color",
    "ink": {"$value": "#0f172a"},
    "paper": {"$value": "#ffffff"},
    "surface": {
      "$value": "#ffffff",
      "$description": "Cards and sheets",
      "$extensions": {
        "omnistate": {"modes": {"light": "#ffffff", "dark": "#0f172a"}}
      }
    },
    "text": {
      "$value": "{color.ink}",
      "$extensions": {
        "omnistate": {"modes": {"light": "{color.ink}", "dark": "{color.paper}"}}
      }
    },
    "shadow": {"$value": "#0f172a33"}
  },
  "spacing": {
    "$type": "dimension",
    "sm": {"$value": "8px"},
    "md": {"$value": "16px"},
    "gutter": {"$value": "{spacing.md}"}
  },
  "radius": {
    "$type": "dimension",
    "md": {"$value": "8px"},
    "card": {"$value": "{radius.md}"}
  },
  "typography": {
    "$type": "typography",
    "body": {
      "$value": {
        "fontFamily": "Inter",
        "fontSize": "16px",
        "fontWeight": 400,
        "lineHeight": 1.5,
        "letterSpacing": "0px"
      },
      "$extensions": {
        "omnistate": {"fontFiles": ["attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd"]}
      }
    }
  },
  "shadow": {
    "$type": "shadow",
    "card": {
      "$value": {
        "color": "{color.shadow}",
        "offsetX": "0px",
        "offsetY": "4px",
        "blur": "12px",
        "spread": "-2px"
      }
    }
  }
}
```

## Groups and types

Tokens sit in five groups at the top of the tree, grouped by type. A group may
be left out, and groups may nest (`color.brand.500` is the token `500` in the
group `brand` of `color`). Any other top-level group is refused.

| Group | Holds | `$type` |
| --- | --- | --- |
| `color` | colors | `color` |
| `spacing` | spacing | `dimension` |
| `radius` | corner radii | `dimension` |
| `typography` | text styles | `typography` |
| `shadow` | shadows | `shadow` |

A token is an object with a `$value`. Its type is the `$type` on the token, or
else the one on the nearest group above it, or else the type of its top-level
group. A `$type` that is not the type of the token's top-level group is
refused, and so is a `$value` that does not fit the type:

- A `color` is `"#rrggbb"` or `"#rrggbbaa"`.
- A `dimension` is a length in pixels, written as text: `"16px"`, `"-2px"`,
  `"0.5px"`.
- A `typography` value is an object of `fontFamily` (a name, `"Inter"`),
  `fontSize` (a dimension of 0 or more), `fontWeight` (a number from 1 to
  1000), `lineHeight` and `letterSpacing` (a dimension). `fontFamily`,
  `fontSize` and `lineHeight` are required.
- A `shadow` value is an object of `color`, `offsetX`, `offsetY`, `blur` (0 or
  more) and, optionally, `spread`. `color` is a color and the rest are
  dimensions.

`$description` and `$deprecated` may sit on the root, groups and tokens, and
other vendors' `$extensions` are kept. Reading a tree and writing it back
changes nothing.

Token and group names are what a design can write after `$`: letters, digits,
`-` and `_`. A name with a space or a dot, or one that starts with `$`, is
refused.

## References

Inside a token file, a token refers to another as `{color.surface}`: the whole
`$value`, the whole value of a mode, or one field of a shadow or typography
value (`"color": "{color.shadow}"`, `"fontSize": "{spacing.md}"`). The token
referred to must exist, must be a token and not a group, and must have the type
that the place needs (a color where a color goes).

In design tags (`docs/formats/design-tags.md`) and in the JSON of design nodes,
the same token is written `$color.surface`: `fill="$color.surface"`. Token
files never use that form: `"$value": "$color.ink"` is refused.

A reference that points at an unknown token, and a loop of references (a token
that refers to itself, or two or more tokens that refer to each other) are
refused, in every mode; the message names the token and, for a loop, the mode.
The `$value`s alone must not loop either, even where a token on the loop has
mode values that end every mode's chain.
`TokenTree::resolve` follows the references of a token in one mode, composite
fields included, and gives its value.

## Modes

The root lists the design system's modes in `$extensions.omnistate.modes`, an
array of at least one distinct name made of letters, digits, `-` and `_`:
`["light", "dark"]`, or one mode per brand. The order is kept.

A token that has one value in every mode states it as `$value`. A token that
has a value per mode also states its value in each mode in the object
`$extensions.omnistate.modes` on the token, as `surface` does in the example
above. That object has a value for every listed mode and for no other mode, and
each value fits the token's type and may be a reference. The token's `$value`
is the value that tools which know no modes read.

`omni tokens export --per-mode --out <dir>` writes one file per mode,
`<dir>/<mode>.tokens.json`. In that file each token's `$value` is its value in
the mode, references stay references, no token has per-mode values, and the root
lists that one mode. It is a valid tree of its own.

## Font files and line heights

A typography token lists the font files it uses in
`$extensions.omnistate.fontFiles`, an array of `attachment:<id>` entries (files
uploaded with `omni attach`). A URL or any other text is refused. At the API
every entry must also be a font that is attached to the workspace: a file that
is unknown, or that is not a font, is refused with its ID.

Every typography value has a definite `lineHeight`: a number, a multiple of the
font size, also in each mode's value. A value without one, or with `"normal"`,
is refused, so text that uses a typography token never falls back to the font's
normal line height (Q44).

## Rebasing

A design system change set that is behind the mainline rebases like any other
(`omni changeset rebase`). Its token changes since the draft's base, each token
it added, changed or removed, are replayed onto the mainline tree, and a token
that only the mainline changed keeps its mainline value. A token that both
sides changed differently is a conflict named by its path (`color.surface`),
with each side's token as JSON, and keeps its mainline value. When the merged
tree fails the checks above, the mainline tree stays and every token that the
change set changed is a conflict.

The components of the design system (`docs/formats/design-tags.md`) are replayed
the same way, component by component: one that only a side changed takes that
side's state, and one that both changed differently is a conflict named by the
component's node ID that keeps its mainline state.

## Components in a design system version

A saved version of the design system holds the token tree above, and the
design system's components under the top-level key `$components`, a list of
`component` nodes in the JSON form of `omni_core::design` (a token tree has no
key of its own that starts with `$`). A design version records the ID of the
design system version it was saved against, so its instances resolve against
the components of that version.
