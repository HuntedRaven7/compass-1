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
6. **corral's layer builder pulls a `localhost/` reference.** Asking for a test
   user derives an image layer, and that builder pulls the base unconditionally
   — so a locally built image dies at exit 3 against `https://localhost/v2/`.
   corral already guards exactly this in `pkg/bootc/local.go` (`isLocalRef`,
   with a comment saying a locally built image is the whole point); the guard is
   missing from `pkg/vmtest/image.go`. Worked around by creating the account in
   our own image: root SSH comes from
   `bootc install --root-ssh-authorized-keys` with no layer involved, so nothing
   is lost. Two-line upstream fix, and the second corral bug this tier has found
   on locally built bootc images.
7. Two of my own diagnostics were unreadable — an ANSI-blind grep that
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

**Update: this was almost certainly a real bug, not a flake.** It recurred, and
the failing line was `df -h /var/lib/containers/storage` — a diagnostic that
prints a number and nothing else, run unprivileged under `bash -e`. `statfs`
needs search permission on every *parent* of its argument, and
`/var/lib/containers` is root-only `0700`, so the df fails with `Permission
denied` even though the graphroot beneath it is readable. Whether it fails
depends on whether that parent already existed with that mode or was created by
the job's own `mkdir -p` under a 022 umask, which is exactly the shape of an
intermittent failure. Measured rather than reasoned: with the parent at 0700 an
unprivileged `df` on a 0700 child gets EACCES; at 0755 the same `df` succeeds.
It now runs under sudo.

The lesson is not about df. It is that a diagnostic step inside `bash -e` has the
same power to fail a 40-minute job as the assertion it was added to explain, and
this one did — which is also why the earlier failure looked like it came from
"a change to a step that runs after corral exits".

### And the marker was racing GDM

With the `df` fixed, the next run got all the way through and failed differently:
`--require-paint` at a framebuffer deviation of **exactly 0.0000**, with
readiness reached at 29s on `Started .*gdm\.service`.

That number is the tell. corral captures its final frame at the instant of
readiness and does not retry, and `Started gdm.service` fires as GDM takes the
DRM device and blanks it — several seconds before the greeter composites
anything. The boot frames in that run measured 0.0227–0.0487 (plymouth's text
console); the ready frame measured zero. So the earlier green run, which this
ADR cited above as "stddev ~0.36, the desktop is up and drawing", passed a race
rather than an assertion. The measurement was real; the conclusion drawn from it
was not safe.

SSH also did not answer in the 60 seconds after that marker
(`kex_exchange_identification: Connection reset by peer`), which matters more
for step 2 than for the control: no SSH means no checks.

Both are fixed by keying readiness off a **state** instead of a unit starting.
`packaging/vmtest/wait-graphical.sh` is a systemd oneshot in both test images
that waits until logind reports a session of type wayland or x11, settles, and
prints `COMPASS-VMTEST: graphical session up` to the serial console. It is
installed in the control image too — the one piece of OS content that image adds
beyond stock Bluefin, and it observes rather than changes anything.

The settle is the single duration in the tier, and it is deliberate rather than
an oversight of this ADR's own "key off markers, never durations" rule: a
session registers with logind before its compositor draws, and *nothing inside
the guest can observe "has painted"* — the only observer of the framebuffer is
corral, on the other side of QEMU, and it exposes no wait for it. Ten seconds is
slack after a state, not an assertion phrased as a duration. If a real paint
gate ever becomes available, it replaces this.

### The readiness marker worked; sshd was not running

The first run with the new marker did exactly what it was built to do — logind
reported a session 2s after the unit started, it settled, and the run reached
ready at 52.8s with **`--require-paint` passing**. So a GNOME desktop with our
Flatpak in it boots and paints under QEMU on a hosted runner, which is the whole
premise of the tier.

It then failed at exit 8: *the checks could not run: SSH never answered*. The
error underneath, in both jobs, was

    kex_exchange_identification: read: Connection reset by peer

which reads like a broken sshd and is not one. Bluefin is a desktop image and
does not enable sshd; QEMU's user-mode hostfwd accepts the connection on the
host and the guest resets it because nothing holds port 22. "Refused" would have
said it plainly — the reset is an artifact of the forward, and it is why this
looked like a protocol problem for two runs.

Both test images now enable sshd, and `wait-graphical.sh` reports sshd's enabled
and active state and what is listening on 22 to the console *before* the marker.
That ordering is the point: everything corral can ask the guest goes over SSH,
and its last copy of the serial log is taken before it waits for SSH — so when
SSH is the thing that is broken, the answer has to already be in the log.

## Step 2: our software is now in the image

The first job tests stock Bluefin and always will — it is the control, and it is
what makes a failure in the second job attributable to us rather than to GNOME,
QEMU or a hosted runner. Alongside it, `boot-compass` boots Bluefin with the
compass Flatpak layered in and a user logged in.

Three decisions in that image are worth recording, because each had an obvious
alternative that is wrong:

- **The Flatpak goes in a named extra installation under `/usr`, not the system
  installation.** A bootc image's `/var` is not image content: it is seeded once
  at install time and is machine state afterwards. `flatpak install --system` at
  build time writes to `/var/lib/flatpak`, which works until a rebase and then
  silently keeps the old app — which is why Universal Blue install Flatpaks from
  a first-boot service instead. A first-boot service would need network in the
  guest and would race the session under test, so the app lives in
  `/usr/lib/compass-flatpak`, declared through `/etc/flatpak/installations.d`.
  `flatpak run` finds it without being told.
- **The runtime is pulled from Flathub during the image build, never in the
  guest.** The bundle carries the app only. Resolving the runtime on the runner,
  where there is network, keeps the VM offline at the point where a network
  failure would look like a product bug.
- **GDM autologin, not a greeter.** A greeter boots, answers SSH and paints, so
  it passes `--require-paint` while telling us nothing: `session.type`,
  `dbus.session` and `portal.desktop` are all meaningless at a login screen. The
  account itself comes from corral's `--user compass` layer and GDM's config
  comes from ours; the two halves are written independently and meet at boot.

The assertions live in `packaging/vmtest/checks.sh`, baked into the image rather
than pushed as `--check` one-liners, so that `bash -n`, shellcheck and a Python
compile run over them in tier 1 (`.github/workflows/shell.yaml`) rather than
30 minutes into a nightly VM run. Only three doctor checks are gated on —
`session.type`, `dbus.session`, `portal.desktop` — because those are the facts
about the target platform that no other tier can establish. Everything else,
including `portal.global-shortcuts` (Spike A's subject) and
`gnome.shell-extension` (we ship none, per ADR-0004), is recorded as evidence
and gates nothing.

Not done, and not pretended to be: the GNOME Shell extension #18 also lists does
not exist in this repository, so there is nothing to install.

## What would change our mind

- If corral's QMP key injection cannot produce Super+Space in practice — `meta_l` is passed through
  to QEMU unvalidated, so this is expected to work but has **not been run yet** — the hotkey half of
  Spike A needs another mechanism, though the portal-binding half still works over SSH.
- If hosted runners lose KVM, the tier moves to a self-hosted runner rather than being abandoned;
  the harness is unchanged either way.
