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

- **A dependency on a young tool, at an unreleased commit.** `vmtest` is not in any tagged corral
  release: v0.6.0 (2026-08-06) has no `cmd/vmtest.go`, and the command exists only on `main`. Our CI
  therefore pins a specific commit, and `go install ...@latest` is *wrong* here — it silently
  installs a corral that fails with `unknown command "vmtest"`, which is how this job failed on its
  first run. The pin should become a tag as soon as one contains the command.

  Mitigated by it being the same organisation's tool, and by its input (a bootc image) and outputs
  (exit codes, PNGs, JSON) all being standard — if corral went away, the image and the assertions
  survive and only the driver is rewritten. But depending on an unreleased feature is a real cost
  and worth stating rather than discovering.
- **llvmpipe software rendering** makes a GNOME session slow and its timing variable. Any assertion
  phrased "within N seconds" will flake; assertions must key off markers.
- **`sudo` in CI**, because `bootc install` partitions a disk and installs a bootloader.
- **Screenshot diffing against stored references is not adopted**, and should not be: it breaks on
  every font, theme and Bluefin update. Screenshots are evidence for humans; `--require-paint` is
  the only pixel assertion gated on.

## What the first working run measured

Stood up in #20. Everything below is from CI, not from documentation:

| | |
|---|---|
| Boot | **26.8 s** guest boot; ~8 min total job with a warm runtime cache, ~35 min cold |
| Image | Bluefin 44.20260908, kernel 7.1.8-200.fc44, installed to disk with `bootc install` |
| Display | `gdm.service` starts every run |
| Painting | frames 1280×800, **stddev ≈ 0.36** against corral's 0.02 blank threshold |

So the two questions this ADR called open are answered: a GNOME desktop **does**
boot and **does** paint under QEMU on a free hosted runner.

Six things had to be fixed to get there, each a real defect rather than a
misconfiguration, and each invisible from reading documentation:

1. `vmtest` is in no corral release — `@latest` installs a corral without it.
2. corral `podman create`s a bootc image to probe its filesystem, and a bootc
   image has no CMD, so it fails on exactly the Universal Blue images the probe
   was written for. Worked around in `packaging/vmtest/Containerfile.bluefin`.
3. Moving podman's graphroot by `storage.conf` hides the image from
   `bootc install`'s privileged container. The bytes must move by bind mount,
   the path must stay canonical.
4. The readiness marker was case-wrong (`Reached target Graphical` vs systemd's
   lowercase unit name).
5. **`graphical.target` is never reached on this image.** `plymouth-quit-wait.service`
   starts and never finishes without a real display, and it gates the target.
   Readiness gates on `Started .*gdm\.service` instead. This is normal here, not
   a fault, and any future marker work should start from this fact.
6. Two of my own diagnostics were unreadable — an ANSI-blind grep that
   under-reported which targets had come up, and a verdict buried under an
   expanded serial-console tail. Both are fixed; the verdict now prints last.

### Stability is not yet established

One caveat, stated because it would be easy to present this as cleaner than it
is: the run immediately before the green one used the **same** corral invocation
and failed. The difference between them was a change to a diagnostic step that
runs *after* corral exits, which should not be able to affect the result, and I
have not explained it.

Until that is understood, or until the job has simply run green many times, the
tier's reliability is unproven. That is precisely why this ADR says nightly
first and merge-queue only after a couple of stable weeks — a VM job that flakes
into the merge queue blocks everyone. If it proves unstable, the `pull_request`
trigger comes off before anything else.

## What would change our mind

- If corral's QMP key injection cannot produce Super+Space in practice — `meta_l` is passed through
  to QEMU unvalidated, so this is expected to work but has **not been run yet** — the hotkey half of
  Spike A needs another mechanism, though the portal-binding half still works over SSH.
- If hosted runners lose KVM, the tier moves to a self-hosted runner rather than being abandoned;
  the harness is unchanged either way.
