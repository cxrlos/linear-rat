# Keymap

<!-- Generated from the action registry in src/keys/registry.rs. Do not edit by hand: run `UPDATE_KEYMAP=1 cargo test --test keymap_reference`. -->

These are lrat's default keys. Press `?` in lrat to see the keys of the screen you are on, with the ones that cannot run now muted. Keys are written in the vim notation `config.toml` uses, and each table heading names the config table that rebinds its actions; see [Configuration](https://github.com/cxrlos/linear-rat/blob/main/docs/config.md#keys).

The Team Issues list also takes the Issue list keys. While an overlay has a text field, as the pickers, the palette and the search do, a character typed without Ctrl or Alt goes into the field instead, except a picker's number keys.

## Everywhere (`[keys.global]`)

| Action | Keys | Description |
| --- | --- | --- |
| `show-my-issues` | `1` | Show My Issues |
| `show-team-issues` | `2` | Show Team Issues |
| `show-analytics` | `3` | Show Analytics |
| `refresh` | `r` | Refresh now |
| `show-cheatsheet` | `?` | Show every key for this context |
| `show-palette` | `:`, `<C-k>` | Find an action or loaded issue |
| `show-search` | `/` | Search all issues in Linear |
| `quit` | `q` | Quit lrat |

## Issue list (`[keys.list]`)

| Action | Keys | Description |
| --- | --- | --- |
| `move-down` | `j` | Move down |
| `move-up` | `k` | Move up |
| `move-to-top` | `gg` | Move to the top |
| `move-to-bottom` | `G` | Move to the bottom |
| `half-page-down` | `<C-d>` | Move down half a page |
| `half-page-up` | `<C-u>` | Move up half a page |
| `focus-left` | `h` | Focus the pane on the left |
| `focus-right` | `l` | Focus the pane on the right |
| `open` | `<Enter>` | Open the focused issue |
| `copy-identifier` | `yi` | Copy the identifier |
| `copy-url` | `yu` | Copy the URL |
| `copy-branch-name` | `yb` | Copy the branch name |
| `copy-markdown-link` | `ym` | Copy a Markdown link |
| `open-in-browser` | `o` | Open the issue or a pull request |
| `open-link` | `O` | Open another link of the issue |
| `edit-state` | `s` | Change the workflow state |
| `edit-assignee` | `a` | Change the assignee |
| `edit-priority` | `p` | Change the priority |
| `edit-labels` | `L` | Add or remove labels |
| `edit-estimate` | `e` | Change the estimate |
| `comment` | `c` | Comment in $EDITOR |
| `create-issue` | `n` | Create an issue |
| `close` | `<Esc>` | Close the top overlay or step back |

## Team Issues (`[keys.team-issues]`)

| Action | Keys | Description |
| --- | --- | --- |
| `switch-team` | `t` | Switch the team of Team Issues |
| `filter-by-state` | `fs` | Filter by workflow state |
| `filter-by-assignee` | `fa` | Filter by assignee |
| `filter-by-label` | `fl` | Filter by label |
| `filter-by-cycle` | `fc` | Filter by cycle |
| `filter-by-project` | `fp` | Filter by project |
| `load-more` | `m` | Load the next page of Team Issues |

## Issue detail (`[keys.detail]`)

| Action | Keys | Description |
| --- | --- | --- |
| `move-down` | `j` | Move down |
| `move-up` | `k` | Move up |
| `move-to-top` | `gg` | Move to the top |
| `move-to-bottom` | `G` | Move to the bottom |
| `half-page-down` | `<C-d>` | Move down half a page |
| `half-page-up` | `<C-u>` | Move up half a page |
| `focus-left` | `h` | Focus the pane on the left |
| `focus-right` | `l` | Focus the pane on the right |
| `copy-identifier` | `yi` | Copy the identifier |
| `copy-url` | `yu` | Copy the URL |
| `copy-branch-name` | `yb` | Copy the branch name |
| `copy-markdown-link` | `ym` | Copy a Markdown link |
| `open-in-browser` | `o` | Open the issue or a pull request |
| `open-link` | `O` | Open another link of the issue |
| `edit-state` | `s` | Change the workflow state |
| `edit-assignee` | `a` | Change the assignee |
| `edit-priority` | `p` | Change the priority |
| `edit-labels` | `L` | Add or remove labels |
| `edit-estimate` | `e` | Change the estimate |
| `comment` | `c` | Comment in $EDITOR |
| `create-issue` | `n` | Create an issue |
| `close` | `<Esc>` | Close the top overlay or step back |

## Overlays (`[keys.overlay]`)

| Action | Keys | Description |
| --- | --- | --- |
| `pick-1` | `1` | Pick entry 1 of the list |
| `pick-2` | `2` | Pick entry 2 of the list |
| `pick-3` | `3` | Pick entry 3 of the list |
| `pick-4` | `4` | Pick entry 4 of the list |
| `pick-5` | `5` | Pick entry 5 of the list |
| `pick-6` | `6` | Pick entry 6 of the list |
| `pick-7` | `7` | Pick entry 7 of the list |
| `pick-8` | `8` | Pick entry 8 of the list |
| `pick-9` | `9` | Pick entry 9 of the list |
| `scroll-down` | `j`, `<Down>`, `<C-n>` | Scroll down |
| `scroll-up` | `k`, `<Up>`, `<C-p>` | Scroll up |
| `toggle-entry` | `<Tab>` | Mark or unmark the highlighted entry |
| `accept` | `<Enter>` | Choose the highlighted entry |
| `close` | `<Esc>` | Close the top overlay or step back |

## Analytics (`[keys.analytics]`)

| Action | Keys | Description |
| --- | --- | --- |
| `scroll-down` | `j`, `<Down>`, `<C-n>` | Scroll down |
| `scroll-up` | `k`, `<Up>`, `<C-p>` | Scroll up |
| `shorter-range` | `[` | Show a shorter range |
| `longer-range` | `]` | Show a longer range |
| `toggle-analytics-view` | `v` | Show Personal or Team analytics |
| `switch-analytics-team` | `t` | Pick the team for Team analytics |
