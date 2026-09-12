# ADR-0004: The GNOME Shell extension ships via extensions.gnome.org, and is baked into Bluefin

**Status:** Accepted · **Date:** 2026-09-12 · Relates to: PLAN.md §3.4–3.6, §10.7 · Blocks Phase 3

## Context

On GNOME 50/51 a Shell extension is the only mechanism for window switching, clipboard history and
paste — Mutter implements none of the relevant Wayland protocols, and
`org.gnome.Shell.Introspect.GetWindows` is allowlisted to the portal backends. See REFERENCES.md §3.

But our first target is a Flatpak on Bluefin, and a sandboxed Flatpak cannot write
`~/.local/share/gnome-shell/extensions/`. The feature exists, the transport exists, and until this
is decided the delivery mechanism does not.

Three options were on the table: publish on extensions.gnome.org; request
`--filesystem=~/.local/share/gnome-shell/extensions`; bake it into the Bluefin image.

## Decision

**Both (1) and (3), and explicitly not (2).**

- Publish the extension on **extensions.gnome.org** as the general answer for every GNOME user. The
  launcher deep-links to it on first run when it detects the extension is absent.
- **Bake it into the Bluefin image** for our first target, so the platform we care most about has
  zero friction. Bluefin is composed in CI from an OCI image, so this is a build-time addition.
- **Do not request the filesystem hole.** Writing into a directory GNOME owns, from inside a
  sandbox, to install code that runs in the compositor's own process, is the kind of permission that
  should be refused — and Flathub reviewers would be right to question it. A launcher that installs
  compositor extensions behind the user's back is a bad neighbour regardless of whether it works.

## Consequences

- Non-Bluefin GNOME users get one manual step at first run. Acceptable: it is the flow every GNOME
  extension uses and users recognise it.
- Version skew between launcher and extension becomes normal, not exceptional. This is why the DBus
  contract is explicitly versioned (PLAN.md §3.5.3) and why `compass-shell` must degrade rather than
  fail. Both are now requirements rather than nice-to-haves.
- `vicinae doctor` must distinguish absent / version-mismatch / present, and say exactly what is
  degraded in each case.
- We own an extensions.gnome.org review cycle on every GNOME release. Budget it.

## What would change our mind

If GNOME ships a portal or protocol for window listing and activation that a sandboxed app may call,
this entire ADR becomes obsolete and the extension can be retired. Watch for that; it is the only
thing that removes the recurring cost.
