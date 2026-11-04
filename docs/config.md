# Configuration

lrat reads one file, `config.toml`, from the XDG config directory: `$XDG_CONFIG_HOME/lrat/config.toml`, which is `~/.config/lrat/config.toml` by default, on macOS too. The file is optional; every setting has a default. It never holds the API key.

## Errors and warnings

A problem in the file shows as a notice above the bottom bar, naming the file and line, and is written to the log. The next key press clears the notice. lrat still starts:

- A syntax error, or a value of the wrong type or out of range, leaves only the section it is in at its defaults. A table that appears twice leaves that table at its defaults. In `[keys]`, a syntax error leaves every context at the default keys, and a bad binding leaves only its context table (`[keys.list]`, for example) at the default keys.
- A syntax error before the first table header leaves the whole file at its defaults.
- An unknown key or table is ignored with a warning, so a misspelt `[key.global]` is reported instead of silently doing nothing.

## `[ui]`

| Key | Values | Default | Effect |
| --- | --- | --- | --- |
| `icons` | `"unicode"`, `"nerd"` | `"unicode"` | The glyphs for workflow state types and priorities. `"nerd"` needs a patched [Nerd Font](https://www.nerdfonts.com/). |
| `wide-layout-columns` | 0 to 65535 | `120` | From this terminal width on, My Issues shows the focused issue's detail beside the list. Below it, the focused pane fills the screen. |

In both icon sets every workflow state type and every priority has its own glyph, so none is told apart by color alone:

| | Unicode | Nerd Font (Font Awesome name) |
| --- | --- | --- |
| Triage | `◇` | `inbox` |
| Backlog | `◌` | `circle_o_notch` |
| Unstarted | `○` | `circle_o` |
| Started | `◐` | `adjust` |
| Completed | `●` | `check_circle` |
| Canceled | `⊘` | `times_circle` |
| Urgent | `!!!` | `exclamation_circle` |
| High | `▮▮▮` | `angle_double_up` |
| Medium | `▮▮▯` | `angle_up` |
| Low | `▮▯▯` | `angle_down` |
| No priority | `···` | `minus` |

## `[theme]`

| Key | Values | Default |
| --- | --- | --- |
| `name` | a built-in theme, or the name of a file in `themes/` | `"terminal"` |

`terminal`, the default, uses the terminal's own palette (a cyan accent, dark gray for what is muted) and leaves the terminal's background alone. The other built-in themes are Base16 schemes ported from ratatoist; each paints its own background (`base00`) behind the whole screen, because its colors are chosen to be read on it:

`catppuccin-mocha`, `dracula`, `everforest-dark`, `gruvbox-dark`, `kanagawa-wave`, `nord`, `one-dark`, `rose-pine`, `solarized-dark`, `tokyo-night`.

### Your own theme

Put a Base16 scheme in JSON next to the config file, at `~/.config/lrat/themes/<name>.json`, and set `name = "<name>"`. A name is letters, digits, `-` and `_`. A built-in theme wins over a file of the same name.

```json
{
  "name": "Mine",
  "base00": "1e1e2e", "base01": "181825", "base02": "313244", "base03": "45475a",
  "base04": "585b70", "base05": "cdd6f4", "base06": "f5e0dc", "base07": "b4befe",
  "base08": "f38ba8", "base09": "fab387", "base0A": "f9e2af", "base0B": "a6e3a1",
  "base0C": "94e2d5", "base0D": "89b4fa", "base0E": "cba6f7", "base0F": "f2cdcd"
}
```

All sixteen colors are required, as six hex digits with or without a leading `#`; other fields are ignored. When there is no such file, lrat uses `terminal` and the notice names the `name` line of `config.toml`. When the file is not a valid scheme, lrat uses `terminal` and the notice names the theme file and the line of the problem.

The slots map to what lrat draws like this:

| Role | Slot |
| --- | --- |
| Background | `base00` |
| Text | `base05` |
| Muted: borders, identifiers, field names, unavailable keys | `base03`, lightened toward `base05` until its contrast with `base00` is at least 3:1 |
| Accent: usable keys, the focused pane, the active cycle | `base0C` |
| Errors | `base08` |
| Triage, started, completed | `base0E`, `base0A`, `base0B` |
| Unstarted | text |
| Backlog, canceled | muted |
| Urgent, high, medium, low | `base08`, `base09`, `base0A`, `base0D` |
| No priority | muted |
| Focused issue row | text and background swapped |

Every Base16 built-in theme keeps a contrast ratio of at least 4.5:1 for text and 3:1 for every other role against its background; a test checks it. `terminal` uses the terminal's own palette, so its contrast depends on the terminal's colors and is not checked.

## `[keys]`

One table per context: `global`, `list`, `detail` and `overlay`. In each, an action's id maps to its keys in vim notation (`"j"`, `"gg"`, `"<C-d>"`, `"<S-Tab>"`, `"<F1>"`). The `?` cheatsheet lists every action.

While an overlay has a text field, as the pickers, the palette and the search query do, a character typed without Ctrl or Alt goes into the field and Backspace deletes, whatever they are bound to. The one exception is a picker's number keys (`pick-1` to `pick-9`), which pick an entry while the filter leaves that many. Keys with Ctrl or Alt and named keys such as `<Enter>`, `<Down>` and `<Esc>` still run their `overlay` actions.

```toml
[keys.global]
quit = "Q"                             # replace: Q is now quit's only key
show-cheatsheet = { append = ["<F1>"] }

[keys.list]
move-down = { prepend = ["n"] }        # n first, j still works
noop = ["gg", "<C-d>"]                 # these keys do nothing, and leave every hint
```

## `[sync]`

lrat polls Linear for changed issues on these intervals, and `r` polls at once. You count as idle after two minutes without a key press. `live` is read and checked now; live updates use it once they land.

Between sessions, lrat keeps what it loaded in a snapshot under `$XDG_CACHE_HOME/lrat/` (`~/.cache/lrat/` by default, on macOS too), one file per workspace and viewer, readable only by you and never holding the API key. The next launch shows it before the first request returns. When Linear cannot be reached at launch, the snapshot stays on screen and lrat loads My Issues again every poll interval (or at once with `r`); it polls only after that load confirms the workspace and viewer. Deleting the directory is always safe.

| Key | Values | Default | Effect |
| --- | --- | --- | --- |
| `active-poll-seconds` | 5 to 3600 | `15` | Seconds between polls while you are using lrat. |
| `idle-poll-seconds` | 5 to 3600, not less than `active-poll-seconds` | `60` | Seconds between polls while you are idle. |
| `pause-when-unfocused` | `true`, `false` | `true` | Stop polling while the terminal reports focus lost. |
| `live` | `true`, `false` | `false` | Also apply live updates from Linear's websocket. |

## `[analytics]`

Read and checked now; the Analytics screen uses it once it lands.

| Key | Values | Default | Effect |
| --- | --- | --- | --- |
| `default-range-weeks` | `4`, `12`, `26` | `12` | How many weeks the Analytics screen opens on. |
| `stale-days` | 1 to 365 | `14` | Days without an update that make an open issue a stale issue. |

## Everything at once

```toml
[ui]
icons = "nerd"
wide-layout-columns = 140

[theme]
name = "nord"

[keys.list]
move-down = { prepend = ["n"] }

[sync]
active-poll-seconds = 15
idle-poll-seconds = 60
pause-when-unfocused = true
live = false

[analytics]
default-range-weeks = 12
stale-days = 14
```
