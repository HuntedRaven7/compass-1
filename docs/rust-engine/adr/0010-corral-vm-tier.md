# ADR-0010: The VM test tier is corral, and it runs on hosted runners

**Status:** Accepted · **Date:** 2026-09-12 · Supersedes the harness sketch in PLAN.md §8.9 ·
Relates to: Phase 0 Spikes A and B

## Context

Three things this project needs could not be tested anywhere we had:

1. **Spike A** — does the GlobalShortcuts portal actually bind Super+Space on real GNOME, and does
   the compositor deliver the keypress? `compass-portals` is written to make the answer legible, but
   no amount of local testing can produce it.
2. **Spike B** — do Landlock and seccomp work inside a Flatpak? Phase 4's sandbox design is unproven
   until this is answered.
3. **The Flatpak manifest**, which has been syntax-validated and never built.

All three were filed as "blocked on someone with access to hardware", and Phase 4 was explicitly not
to be designed further until Spike B answered. That is a bad place for a plan to sit.

PLAN.md §8.9 proposed building a harness for this: `bootc-image-builder` to a qcow2, boot under
QEMU, provision over SSH, drive a scripted session, capture screenshots. It also asserted that
GitHub-hosted runners expose no `/dev/kvm`, and on that basis proposed Depot CI sandboxes or a
self-hosted runner.

## Decision

**Adopt [`tuna-os/corral`](https://github.com/tuna-os/corral) as the VM tier**, and run it on
GitHub-hosted `ubuntu-24.04`.

`corral vmtest` is the harness §8.9 described, already built: it layers test-only customisation over
a published bootc image without modifying it, builds the disk with `bootc install`, boots it, waits
on a serial-console marker, runs assertions, and writes serial log, per-interval screenshots, a
timelapse, `result.json` and diagnostics.

## Why this rather than our own

- **Bluefin is a bootc image and corral's input is a bootc image.** No conversion step to own, and
  the VM under test is the target built the way the target is built.
- **`--require-paint`** fails a run whose final frame has luminance standard deviation ≤ 0.02. For a
  *launcher*, "the session came up and drew nothing" is the most likely serious regression and the
  one an SSH probe reports as success. We would have had to invent this check; here it is a flag.
- **Exit codes are per failure class** — 2 (this host cannot run it) is distinct from 6 (guest never
  ready) and 9 (painted nothing). This is what stops the tier becoming a check people ignore.
- **`corral key` / `corral type` / `corral screenshot`** inject scancodes at QEMU's emulated keyboard
  over QMP. This is the part we could not have easily replaced, and it is what makes Spike A a CI
  job: a global hotkey synthesised inside the session under test proves nothing, whereas a scancode
  at the emulated keyboard is indistinguishable from a user pressing the key.
- It is **the same organisation's tool**, already used as the boot gate for tunaOS, so the
  maintenance relationship is not arm's length.

## The KVM claim in §8.9 was wrong, and it was load-bearing

§8.9 asserted hosted runners have no `/dev/kvm`, citing a community discussion from 2022, and
designed Depot and self-hosted-runner options around it. corral's own docs claim the opposite. Since
the answer decided whether this tier needed infrastructure we do not have, it was measured rather
than argued — a throwaway probe workflow, since deleted
([run 34686387919](https://github.com/tuna-os/compass/actions/runs/34686387919)):

| Runner | `/dev/kvm` | `kvm-ok` | QEMU accelerators |
|---|---|---|---|
| `ubuntu-24.04` (x86_64) | present, `nested=1` | "KVM acceleration can be used" | `tcg kvm` |
| `ubuntu-24.04-arm` | **absent** | "does not exist" | compiled in, unusable |

The tier therefore costs no new infrastructure, and the Depot/self-hosted options are dropped. It is
**x86_64 only**; arm64 would fall back to TCG, and Bluefin's primary target is x86_64 anyway.

The general lesson is the one ADR-0006 already recorded: a platform limitation taken from a
three-year-old forum thread is a guess. This one had shaped a whole section of the plan.

## What it unblocks, and what it does not

Unblocks: Spikes A and B become CI jobs rather than favours asked of a human with a Bluefin laptop;
the Flatpak manifest finally gets built; and the C++ engine can be exercised on the real target to
produce the behavioural baseline the Rust parity suites compare against — which is worth more than
it sounds, since today those suites compare against *our reading* of the C++ source.

Does not unblock: **the Rust engine has no UI yet.** There is no `compass-ui` crate, so for now this
tier can only test the C++ engine and the portal/Flatpak/sandbox questions. That is not a reason to
defer it — the baseline and the two spikes are precisely what is needed before the UI exists — but
it does mean the tier's first job is not "test the Rust launcher".

## Costs

- **A dependency on a young tool.** Mitigated by it being the same organisation's, and by the fact
  that its input (a bootc image) and its outputs (exit codes, PNGs, JSON) are all standard — if
  corral went away, the image and the assertions survive and only the driver is rewritten.
- **llvmpipe software rendering** makes a GNOME session slow and its timing variable. Any assertion
  phrased "within N seconds" will flake; assertions must key off markers.
- **`sudo` in CI**, because `bootc install` partitions a disk and installs a bootloader.
- **Screenshot diffing against stored references is not adopted**, and should not be: it breaks on
  every font, theme and Bluefin update. Screenshots are evidence for humans; `--require-paint` is
  the only pixel assertion gated on.

## What would change our mind

- If corral's QMP key injection cannot produce Super+Space in practice — `meta_l` is passed through
  to QEMU unvalidated, so this is expected to work but has **not been run yet** — the hotkey half of
  Spike A needs another mechanism, though the portal-binding half still works over SSH.
- If hosted runners lose KVM, the tier moves to a self-hosted runner rather than being abandoned;
  the harness is unchanged either way.
