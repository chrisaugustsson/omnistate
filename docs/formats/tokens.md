# Design tokens

A project's design system doc holds its design tokens as one JSON tree in the
W3C Design Tokens (DTCG) format, with one value per mode (Q27). The checks in
`omni_core::design::tokens` (`TokenTree::from_json`) decide what a tree may
say; the API runs them on every import, so a tree that is stored always passes.
A project has at most one design system doc, and `omni tokens import` replaces its whole token tree in one batch.

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
  },
  "state": {
    "isOpen": {
      "$type": "boolean",
      "$value": false,
      "$extensions": {"omnistate": {"codeSyntax": {"css": "--is-open", "swift": "isOpen"}}}
    },
    "count": {"$type": "number", "$value": 2},
    "total": {"$type": "number", "$value": "{state.count}"},
    "cta": {
      "$type": "string",
      "$value": "Pay now",
      "$extensions": {"omnistate": {"scopes": ["content"]}}
    }
  }
}
```

## Groups and types

Tokens sit in five groups at the top of the tree, grouped by type. A group may
be left out, and groups may nest (`color.brand.500` is the token `500` in the
group `brand` of `color`). A top-level group with any other name holds the
state of a prototype: numbers, strings and booleans (see below).

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

- A `number` is a JSON number: `2`, `0.5`.
- A `string` is a JSON string: `"Pay now"`.
- A `boolean` is `true` or `false`.

A top-level group other than the five above may state no `$type`, or one of
`number`, `string` and `boolean`. In a group without a `$type` each token states
its own `$type`, and it is one of those three: a token without a `$type`, or a
`color`, `dimension`, `typography` or `shadow` token, is refused. A token
refers to another of the same type as `{state.count}`; a string that refers to
a number token is refused like any reference to the wrong type.

## Scopes and code syntax

A token of any type may state, in `$extensions.omnistate`, the fields it may
bind to and its name per platform:

- `scopes` is an array of `fill`, `stroke`, `textColor`, `content`, `visible`,
  `opacity`, `width`, `height`, `gap`, `pad`, `radius`, `fontSize`,
  `lineHeight`, `letterSpacing` and `strokeWidth`. A name that is not one of
  them is refused. A token without `scopes` may bind to any field of its type.
- `codeSyntax` is an object that maps a platform name to a string, such as
  `{"css": "--is-open", "swift": "isOpen"}`. `omni export` writes the token to
  `tokens.css` under its `css` name, written as it stands, in place of the name
  derived from its path, and references to the token use that name.

`tokens.css` writes a number as it is, a boolean as `true` or `false` and a
string in quotes (`--state-cta: "Pay now";`). A variable derived from a path
joins the segments with `-`, each in kebab case: `state.isOpen` is
`--state-is-open`.

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
array of at least one distinct name made of letters (of any script), digits, `-` and
`_`: `["light", "dark"]`, `["Mörkt", "Ljust"]`, or one mode per brand. The order is kept.

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

## Components in a design system head

The head of the design system holds the token tree above, and the design
system's components under the top-level key `$components`, a list of
`component` nodes in the JSON form of `omni_core::design` (a token tree has no
key of its own that starts with `$`). A design resolves its instances against
the components of the design system's head when it is read.
