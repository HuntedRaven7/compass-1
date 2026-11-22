# Browser host prototype (#315): threat model and ownership

**Status:** prototype, not shipped · **Relates to:** ADR-0008, ADR-0022 §3, issues #315–#318

This records the narrow proof in `crates/compass-browser-host` and the decisions
#315 asks for. ADR-0022 already chose option 1 of ADR-0008 — a small native
messaging host shipped with the launcher — and delegated the ownership/security
tradeoff. Nothing here changes that choice; #316 replaces the prototype's
`EmptyTabs` source with a client of the engine's IPC socket.

## What was proved

`cargo test -p compass-browser-host` spawns the real `compass-browser-host`
binary and drives `connect` → `getTabs` → `focusTab` over its stdin and stdout
using Chrome's native messaging framing (a little-endian u32 length, then JSON).
The host answers one frame per request, survives an unknown protocol version,
and exits cleanly when the browser closes the pipe. The framing refuses a zero
or >1 MiB length rather than allocating it.

This is the transport proof, not a tab search: the tab source is empty, exactly
as the engine's `NoBrowsers` is today.

## Threat model

| Threat | Boundary | Why it does not apply / how it is met |
|---|---|---|
| Any page on localhost reaches the bridge | network | There is no socket. Native messaging is opened by the browser, for the one extension named in the host manifest. This is why ADR-0022 rejected option 3. |
| A different extension spawns the host | host manifest | `chrome_manifest` / `firefox_manifest` require a specific `chrome-extension://…/` origin or Firefox id; a wildcard is refused. Only the browser's native-host registry decides who may spawn us, so the manifest is trusted input owned by the package. |
| Extension impersonation | native host | The host is spawned with the extension's origin already fixed by the browser; it does not accept a claimed identity in a message. |
| Replay of a captured `focusTab` frame | host | Requests have no side effect beyond the browser's own focus action. #316 must not add a privileged action to this channel without its own consent. |
| Oversized / hostile framing | length prefix | Invalid lengths are rejected before allocation; a bad version or malformed JSON gets an error reply and the host keeps serving. |
| Token theft | — | No pairing token exists, so there is none to store. This is the direct consequence of rejecting the localhost transport. |
| Sandbox escape | OS sandbox | The host is subject to the same Flatpak confinement as the launcher. It does not add a filesystem grant; the manifest is written by the package, not by a running extension (ADR-0004's refusal). |

## Install paths and ownership

| Component | Owned by | Linux path |
|---|---|---|
| `compass-browser-host` binary | launcher package | the same `bindir` as `compass` |
| Chrome manifest | launcher package (generated at build time) | `~/.config/google-chrome/NativeMessagingHosts/org.tunaos.compass.browser.json`, also `~/.config/chromium/NativeMessagingHosts/` |
| Firefox manifest | launcher package (generated at build time) | `~/.mozilla/native-messaging-hosts/org.tunaos.compass.browser.json`; Flatpak Firefox uses `~/.var/app/org.mozilla.firefox/.mozilla/native-messaging-hosts/` |
| WebExtension | the tab-search extension (#317) | browser store or sideload, not the launcher package |

The launcher owns the binary and its manifests; the extension owns its own
script and manifest id. Keeping these in different packages is the point of
ADR-0008: a browser vendor change touches the extension and the host manifest,
not launcher core.

## Lifecycle and upgrades

- The browser spawns the host on demand; there is no daemon and no login item.
- A launcher upgrade replaces the binary in place. The manifest points at the
  absolute path, so after an upgrade the next spawn uses the new binary with no
  manifest edit; a different install prefix changes the path and needs the
  manifest regenerated.
- A stale host from an old version answers an unknown-version `connect` with an
  error rather than guessing; the extension reports it and asks the user to
  reinstall rather than falling back to an unproven transport.
- Flatpak upgrades swap the whole runtime, including the host binary.

## Browser support

ADR-0022 limits support to browsers that expose native messaging to host
binaries inside their sandbox: Firefox through the WebExtensions portal is the
target; Flatpak Chromium-based browsers are out of scope until they do. #318
holds the real Bluefin matrix and the distribution/signing decision. This
prototype makes no claim about any browser it has not been spawned by.

## Out of scope here

- #316: engine IPC for `getTabs`/`focusTab`, production lifecycle and packaging.
- #317: the Raycast-compatible tab search extension over `BrowserExtension/*`.
- #318: the real browser matrix, signing and release evidence.
