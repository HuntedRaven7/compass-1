# Writing Rhai scripts for Compass

A Rhai script is the smallest thing that can put an interactive, searchable list in the launcher: a
folder with a manifest and a `.rhai` file, no Node, no npm, no build step. It runs in-process in a
sandbox that has no filesystem, network, environment or process access unless the manifest asks for
a capability and the host grants it. Why this tier exists, and what it is not, is in
[PLAN.md §2.2](./PLAN.md) and [ADR-0005](./adr/0005-rhai-seam-now-tier-later.md).

| You want… | Use |
|---|---|
| a one-shot command whose output is text | a script command (shell, Python, …) |
| a searchable list with actions, some state, no dependencies | **a Rhai script** |
| React, npm packages, HTTP, OAuth, forms, grids | a TypeScript extension |

The language itself is documented in [the Rhai book](https://rhai.rs/book/). This page covers what
Compass adds and expects.

## Start by copying an example

Six first-party scripts live in [`extensions/rhai-examples/`](../../extensions/rhai-examples/).
Each opens with a comment saying what it demonstrates; pick the closest one and copy the folder.

| Example | Shows | Capabilities |
|---|---|---|
| [`web-search`](../../extensions/rhai-examples/web-search/main.rhai) | the smallest useful script: a table, `url::encode`, `open` actions | `application.open`, `clipboard.write` |
| [`unit-converter`](../../extensions/rhai-examples/unit-converter/main.rhai) | parsing the query, sections, accessories, `copy` actions, an empty state | `clipboard.write` |
| [`epoch-converter`](../../extensions/rhai-examples/epoch-converter/main.rhai) | the `time::` helpers, a detail pane with metadata, `try`/`catch` | `clipboard.write` |
| [`generators`](../../extensions/rhai-examples/generators/main.rhai) | `random::`, sub-commands parsed from the query, `paste` actions | `clipboard.write`, `clipboard.paste` |
| [`quick-notes`](../../extensions/rhai-examples/quick-notes/main.rhai) | persistent state with `storage::`, actions that run script functions, returning effects | `storage.read`, `storage.write`, `clipboard.write` |
| [`system-health`](../../extensions/rhai-examples/system-health/main.rhai) | `process::run` with timeouts and a budget, "unsupported" stated plainly, `filtering: true` so programs run on open and refresh only, a list-wide action | `process.execute`, `clipboard.write` |

```sh
mkdir -p ~/.local/share/compass/scripts
cp -r extensions/rhai-examples/web-search ~/.local/share/compass/scripts/my-search
$EDITOR ~/.local/share/compass/scripts/my-search/main.rhai
```

Scripts are found in `$XDG_DATA_HOME/compass/scripts/<name>/` (usually
`~/.local/share/compass/scripts/`, which Compass creates) and then in `compass/scripts/` under each
`$XDG_DATA_DIRS` entry and beside the installed binary (`../share/compass/scripts`). A script in
your own directory shadows a packaged one with the same folder name. Folders starting with a dot
are ignored.

Each script is a command in root search, found by its title, description and keywords. Enter opens
it: the launcher shows what `search` returns and calls it again as you type. Compass watches the
script directories, so saving a file is enough: an open script re-renders with the new code, a new
folder appears in root search, and a script that no longer compiles says why when opened. The
examples are installed with Compass (`share/compass/scripts`), so they are in root search from the
start.

Every example is also exercised by `cargo test -p compass-script --test examples`; if you change
one, that test says whether it still works.

## The folder

```
my-search/
├── script.toml     the manifest (required)
└── main.rhai       the entry file (the default name)
```

### `script.toml`

```toml
title = "Web Search"                      # required: the root-search title
description = "Search the web"            # the root-search subtitle
icon = "globe"                            # a built-in icon name
keywords = ["google", "duckduckgo"]       # extra root-search terms
capabilities = ["application.open"]       # what the script may reach; see below
entry = "main.rhai"                       # optional; must stay inside the folder
```

Unknown fields are an error, so a typo in a field name is reported rather than ignored. The
script's id is `script.` plus the folder name, which is also the namespace its storage lives in.

The manifest is read **without running the script**: a host can show what a script asks for, and
ask you, before any of its code runs.

## The contract: `fn search(query)`

Every script defines `fn search(query)`. It is called with the text in the search bar, on every
keystroke, and returns what to show. A script without it does not load.

The simplest return value is an array of items:

```rhai
fn search(query) {
    [
        #{ title: "Hello", subtitle: query },
        #{ title: "World", icon: "globe" },
    ]
}
```

Anything richer is a map:

```rhai
fn search(query) {
    #{
        placeholder: "Type a city",
        sections: [
            #{ title: "Favourites", items: [ #{ title: "Lisbon" } ] },
            #{ title: "Everything else", items: [ #{ title: "Oslo" } ] },
        ],
        empty: #{ title: "No cities", description: "Try another name" },
    }
}
```

Return a map with `markdown` instead of `items`/`sections` to show a detail view.

### List fields

| Field | Type | Meaning |
|---|---|---|
| `items` | array of items | rows in an untitled section |
| `sections` | array of `#{ title, subtitle, id, items }` | titled groups, after `items` |
| `title` | text | navigation title |
| `placeholder` | text | search-bar placeholder |
| `empty` | text, or `#{ title, description, icon, actions }` | shown when there are no rows |
| `filtering` | bool, default `false` | `true`: the launcher fuzzy-filters your rows itself, and you can return everything |
| `show_detail` | bool | show the detail pane; defaults to on when any row has a `detail` |
| `loading` | bool | show a spinner |
| `actions` | array of actions | actions for the list as a whole |

### Item fields

| Field | Type | Meaning |
|---|---|---|
| `title` | text, **required** | the row's text |
| `subtitle` | text | secondary text |
| `id` | text | a stable key; set it when rows can move, so the selection follows the row |
| `icon` | text | a built-in icon name |
| `keywords` | array of text | extra terms for the launcher's own filtering |
| `accessory` | text, or `#{ text, tag, color, icon, tooltip }` | one trailing decoration |
| `accessories` | array of the above | several |
| `detail` | markdown text, or `#{ markdown, metadata: [#{ title, text }, …] }` | the right-hand pane; `()` in `metadata` is a separator |
| `actions` | array of actions | the first one runs on Enter |

"Text" accepts strings, numbers, characters and booleans. Any other type, a missing title or an
unknown field is an error naming exactly where it is — for example
`invalid view at sections[1].items[3].title: every item needs a title` — rather than a row that
silently disappears.

### Detail fields

`markdown`, `metadata` (as above), `title`, `loading`, `actions`.

### Actions

An action is a map with a `title` and **exactly one** of:

| Key | Does | Needs |
|---|---|---|
| `copy: text` | copies text, closes the window | `clipboard.write` |
| `paste: text` | pastes text into the focused app, closes the window | `clipboard.paste` |
| `open: url_or_path` | opens it with the default app, closes the window | `application.open` |
| `run: "name"` | calls `fn name()` in your script | nothing |
| `run: "name", args: value` | calls `fn name(value)` | nothing |
| `run: \|\| …` | calls the closure | nothing |

plus, optionally, `id`, `icon`, `style: "destructive"` and `shortcut: "ctrl+shift+c"` (modifiers
`ctrl`, `alt`, `shift`, `meta`; `cmd` means Control, as in the TypeScript tier).

A `copy`, `paste` or `open` action whose capability the script does not hold is refused when the
view is built, with an error naming the capability — not when the user presses Enter.

`run` actions are how a script changes its own state. The function's return value says what the
launcher should do next:

```rhai
fn add(text) {
    let notes = storage::get("notes") ?? [];
    notes.push(text);
    storage::set("notes", notes);
    #{ rerender: true, toast: "Saved", style: "success" }
}

fn search(query) {
    [#{ title: `Add "${query}"`, actions: [#{ title: "Add", run: "add", args: query }] }]
}
```

| Key in the returned map | Effect |
|---|---|
| `rerender: true` | call `search` again with the current query |
| `toast: text` (+ `message`, `style`: `info` / `success` / `failure`) | a transient message |
| `hud: text` | a heads-up message, and the window closes |
| `close: true` | close the window |
| `pop: true`, `pop_to_root: true` | navigate back |

Return `()` (or nothing) for no effect. Closures are convenient for per-row actions, and capture the
row they were made in:

```rhai
for name in names {
    rows.push(#{ title: name, actions: [#{ title: "Say it", run: || #{ hud: name } }] });
}
```

## Permissions

What a script may reach is decided before any of its code runs, from its manifest and where it is
installed:

* **Scripts installed with Compass** — under an `$XDG_DATA_DIRS` entry or beside the binary (the
  Flatpak's `/app/share`, a package's `/usr/share`, an AppImage's own tree) — are granted every
  capability their manifest declares. Installing them was the decision; they are ours or the
  distribution's.
* **Your own scripts**, in `$XDG_DATA_HOME/compass/scripts`, are granted **nothing** until you
  allow it. The first time you open one that declares capabilities, the launcher asks — "Allow
  Quick Notes to: read its saved data, save data, copy to the clipboard" — before a line of it
  runs. **Enter** allows and is remembered; **Escape** refuses, grants nothing, remembers nothing,
  and it asks again next time. A script that declares no capabilities is never asked about.
* A copy of a packaged script placed in your own directory shadows it, and is yours: it is asked
  about like any other.
* Allowing covers the capabilities listed at the time. A script that later starts declaring
  another one is asked about that one when next opened; one it stops declaring is no longer granted.
* Answers are kept in `$XDG_CONFIG_HOME/compass/script-grants.json`
  (`{"scripts": {"script.quick-notes": ["clipboard.write", …]}}`). The launcher's **Script
  Permissions** command lists what each script was allowed and revokes it (Enter); deleting an
  entry, or the file, does the same by hand. It is read again whenever a script is opened, so no
  restart is needed, and a view open on a script whose permissions were revoked closes.

Capability names this build does not know are never granted and never asked about.

## Capabilities

A capability is a line in the manifest that makes a module of functions exist. A script that did
not declare `clipboard.write`, or declared it and was not granted it, has **no** `clipboard::copy`:
the script does not load, with `clipboard is not available to this script`. There is nothing to try
and catch, and nothing to probe.

| Capability | Functions |
|---|---|
| `clipboard.write` | `clipboard::copy(text)` |
| `clipboard.read` | `clipboard::read()` → text or `()` |
| `clipboard.paste` | `clipboard::paste(text)` |
| `application.open` | `application::open(url_or_path)` |
| `storage.read` | `storage::get(key)` → value or `()`, `storage::keys()` → array |
| `storage.write` | `storage::set(key, value)`, `storage::remove(key)` |
| `notification.send` | `notification::send(title, body)` |
| `process.execute` | `process::run(program, args, timeout_ms)` → `#{ code, stdout, stderr, timed_out }`, or `()` when the program is not installed |

Each capability brings only its own functions: `clipboard.write` gives `clipboard::copy` but not
`clipboard::read`. Storage values are anything JSON can hold (strings, numbers, booleans, arrays,
maps, `()`), are private to the script, and survive restarts: they live in Compass's encrypted
extension database, in a namespace of the script's own, so storage needs the login keyring as an
extension's does. The clipboard is the one extensions use (the GNOME Shell extension, or
data-control on wlroots, where `paste` copies and you paste). A host function that fails (the
clipboard is unavailable, say) raises an error the script can `catch`.

`process::run` starts `program` with `args` (an array of text) directly, with no shell, standard
input closed and the launcher's environment; inside the Flatpak it runs on the host, through
`flatpak-spawn --host`. It waits at most `timeout_ms` milliseconds, and never longer than what is
left of the call's own [time limit](#limits): a program still running then is killed and the map
says `timed_out: true`. `code` is `()` when a signal ended the program, and `stdout` and `stderr`
keep the first 256 KiB of each. A program that is not installed returns `()` rather than raising
an error, so a script can probe for a tool — except inside the Flatpak, where it is
`flatpak-spawn` that runs and fails, with a non-zero `code`. A program is run only when the script
calls it, so a script that runs one from `search` and does not set `filtering: true` runs it on
every keystroke; see `system-health`.

Other capabilities in `compass-extension-api` (network, windows, OAuth…) have no Rhai functions
yet. Declaring one is allowed and does nothing.

## Always available

These need no capability; none of them reads anything outside the script.

| Function | Returns |
|---|---|
| `print(x)`, `debug(x)`, `log(text)` | writes to Compass's log, not to a terminal |
| `parse_json(text)` | a value; `map.to_json()` goes the other way |
| `time::now()`, `time::now_ms()` | Unix time in seconds / milliseconds |
| `time::format(seconds)` | RFC 3339 in UTC, e.g. `2023-11-14T22:13:20Z` |
| `time::format(seconds, pattern)` | a [`time` crate format](https://time-rs.github.io/book/api/format-description.html), e.g. `"[day] [month repr:long] [year]"` |
| `time::parse(text)` | seconds, from RFC 3339 |
| `time::parts(seconds)` | `#{ year, month, day, hour, minute, second, weekday, ordinal }` |
| `random::uuid()` | a v4 UUID |
| `random::int(low, high)` | an integer in `low..high` |
| `random::float()` | a float in `0.0..1.0` |
| `url::encode(text)` | percent-encoded text for a query string |

Plus Rhai's pure standard library: arithmetic, logic, strings, arrays, maps, maths, iterators and
function pointers. Absent on purpose: `eval`, `import`, `sleep`, `exit` and `timestamp`.

## Limits

Every call to `search` or to an action runs under these limits. Hitting one stops the call with an
error in the launcher; `try`/`catch` cannot catch it.

| Limit | Default |
|---|---|
| operations per call | 500,000 |
| wall-clock time per call | 2 s |
| call depth (recursion) | 48 |
| expression nesting | 64 (32 inside functions), checked when loading |
| string length | 1 MiB |
| array length, map size | 10,000 |
| variables in scope | 1,000 |

`search` runs on every keystroke, so aim for a small fraction of these. A script runs off the UI
thread, so a slow one makes its own results late but never freezes the launcher.

## Rhai habits that trip people up

* **Functions cannot see variables outside them.** Top-level `const`s are reached as
  `global::NAME`; everything else is passed in. (Top-level code also runs on every call, so keep it
  to constants.)
* **Back-tick strings are raw**: `` `a\nb` `` is four characters. Use `"\n"` in a quoted string, and
  back-ticks for `${interpolation}`.
* **Some string methods change the string in place and return nothing**: `trim()`, `make_lower()`,
  `truncate()`. Copy first: `let q = query; q.trim();`. (`to_lower()`, `to_upper()` return new
  strings.)
* **A `for` variable is a copy.** To change an array's elements, index it:
  `for i in 0..items.len() { items[i].x = 1; }`.
* **`try`/`catch` is a statement**, not an expression: assign inside it.
* **Variables must be declared** before use (strict mode), so a typo is caught when the script
  loads, not when a user reaches that line.
* **Integer division truncates**: `7 / 2` is `3`; write `7.0 / 2.0` for `3.5`.

## For host developers

The tier is the `compass-script` crate; the engine's side is `crates/compass/src/rhai_scripts.rs`
(loading, the grant policy, the view session, hot reload) and `rhai_host.rs` (the real
`ScriptHost`). A script's root entry is `rhai:script.<name>`; the launcher lists them with
`ListRhaiScripts` (IPC v14) and opens one with `RunExtensionCommand`, after which it is followed
and driven with the same `ExtensionView` / `ExtensionEvent` / `ExtensionAlertAnswer` /
`CloseExtension` requests as an extension's view. The search bar's text arrives as the
`rhai.search` handler; a consent prompt is the view's alert. The crate's pieces:

* `discovery::search_paths()` / `scan()` / `load()` — find scripts, with shadowing, as
  `compass_core::manifest::registry` finds Node extensions.
* `ScriptManifest::declare_into(&mut CapabilityRegistry)` — record declarations; the host then
  grants (after asking the user, or by policy).
* `ScriptInstance::load(&script, &registry, host, Limits::default())` — build the engine and
  compile. Capabilities are resolved here, once; after a grant changes, build a new instance.
* `instance.search(query).await` → `compass_extension_api::ViewTree`, the same type the TypeScript
  tier produces.
* `instance.invoke(&ActionRequest).await` → `ActionResponse`. Use the seam's `ActionIndex` and
  `Pending` exactly as for a TypeScript extension.
* `ScriptHost` — the trait a host implements for the capability-backed functions. Every method takes
  a `CapabilityGrant` by value, which only `CapabilityRegistry::check` can produce. `MemoryHost` is
  the in-memory reference implementation used by the tests.
* `ScriptWatcher` + `ScriptSet` — hot reload: a debounced "something changed", and a diff of what
  did.

The negative sandbox tests in `crates/compass-script/tests/sandbox.rs` are the specification of the
sandbox; `tests/seam.rs` renders one fixture through the TypeScript tier's `view_model::to_view` and
through a script and requires the same view.

### Trying a script without the launcher

`crates/compass-script/tests/examples.rs` loads each example with `discovery::load`, grants what
its manifest declares, calls `search` and fires actions against a `MemoryHost`. Copy one of those
tests, point it at your folder, and `cargo test -p compass-script --test examples` shows the view
your script returns — or the exact path of what is wrong with it.
