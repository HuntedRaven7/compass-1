# Browser Tabs

A Raycast-compatible Compass extension that lists the open tabs of connected
browsers and focuses the one you pick. Search is the launcher's own fuzzy
filter: type a title or URL in the search field.

## Building and installing

```sh
npm install
npm run build
npx vici build -o ~/.local/share/compass/extensions/browser-tabs
```

Then open the launcher and run **Search Browser Tabs**. Without a connected
browser the command shows an empty state rather than failing.

## What it needs

Tabs come from the `BrowserExtension/getTabs` extension API. A browser reaches
that API through the launcher-shipped `compass-browser-host` native messaging
host (ADR-0022). No browser connected, no tabs: the empty state explains how to
connect one. This extension does not install browser manifests or spawn the
host — those belong to the package; see
`docs/rust-engine/BROWSER-HOST-PROTOTYPE.md`.

The view uses the runtime's `browserId` field, which the published `Tab` type
leaves out, to group tabs by browser and give them a stable identity.

## Tests

`npm test` bundles `test/*.test.ts` with the SDK's esbuild and runs them with
Node's test runner. The tests cover missing titles and URLs, duplicate titles,
two browsers with deterministic ordering, an empty/disconnected state, and a
focus call that throws.
