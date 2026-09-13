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

## Spike A is wired, and its answer is whatever GNOME says

`vicinae spike global-shortcut` binds a shortcut through the GlobalShortcuts
portal and waits; `scripts/vmtest/spike-a.sh` starts it in the guest, presses
`meta_l spc` at QEMU's emulated keyboard from the host, and reads the report
back. The job leaves the VM up (no `--rm`) because that host-side keypress
cannot happen inside a corral `--check`, and a hotkey synthesised inside the
session under test would prove nothing.

**Nothing about the outcome is gated.** The job fails only if the spike produces
no report. Every result it can record is an answer to the question #3 asks,
including the one that is easiest to mistake for a broken run: GNOME may refuse
to bind without a permission dialog that no CI can click. That would be a real
finding about shipping a launcher on GNOME 50/51 — it belongs in the report, not
behind a red X with no information in it.

One judgement worth stating because it is a heuristic and not a fact:
`trigger_matches_request` compares the wire syntax we send (`SUPER+space`) with
the *display* text GNOME returns (`Super+Space`). Nothing in the specification
requires those to be relatable, so the comparison normalises case, separators
and the usual synonyms and can be fooled. Both strings are in the report
verbatim so a reader never has to trust it.

### The ready marker has to mean "as ready as I will ever be"

The spike printed `SPIKE-A-READY` only on the path where binding succeeded. So a
spike that could not reach the portal wrote its report, exited, and left the
harness waiting out its full 150-second timeout for a line that was never
coming — and then throwing away the answer it already had.

That is the failure mode a spike can least afford, because "the portal was not
reachable" is *itself* one of the answers Spike A exists to record. The marker
now goes out on every path, and the harness waits for the marker **or** the
spike exiting. Either alone is enough to turn a hang into a report.

### A note on `pgrep` in a waiter

Spike A's collector originally waited for `! pgrep -f "spike global-shortcut"`.
That predicate can never become true: `pgrep -f` matches full command lines, and
the shell evaluating it has the pattern in its own. The wait timed out every
time, 180 seconds after a spike that had already finished.

It is written down because the shape recurs — any "wait until my process is
gone" check written with `pgrep -f` and a distinctive-looking string has this
bug, and it presents as a timeout rather than as a mistake. The collector now
waits on a sentinel file the launcher writes, which also carries the exit
status.

### The paint margin is thin, and that is worth watching

The compass image passes `--require-paint`, but not by much: the ready frame in
the Spike A run measured **0.0254** against corral's 0.02 blank threshold, with
the boot frames spanning 0.0227–0.0487. A GNOME session under llvmpipe is mostly
flat dark pixels, so there is far less margin here than the 0.36 an earlier note
in this ADR cited — and that figure has already been shown to have been luck
rather than measurement.

No action yet, and deliberately not a threshold tweak: the blank check is
corral's and lowering it would defeat the one pixel assertion the tier has. But
if `--require-paint` starts flapping on the compass image, this is why, and the
fix is to give the session something to draw rather than to move the line.

## Spike A's first answer

Measured on Bluefin 44 under QEMU, in a real GNOME Wayland session with the
compass Flatpak installed:

```json
{
  "portal": "available (interface v1)",
  "requested_trigger": "LOGO+space",
  "bind_outcome": "error: the portal did not answer `BindShortcuts` within 30s",
  "bound": [],
  "activated": false,
  "waited_seconds": 120.0
}
```

**Two of the three questions are answered, and the third is not.**

1. **Is the GlobalShortcuts portal there?** Yes — `org.freedesktop.portal.Desktop`
   exposes GlobalShortcuts at interface version 1. The premise of
   `compass-portals` holds on the target.
2. **Is binding permitted unattended?** No. `BindShortcuts` did not return
   within 30 seconds.
3. **Does a keypress reach us?** **Unknown, and this run says nothing about
   it.** `activated: false` is not evidence about the keyboard: there was no
   binding for `meta_l spc` to trigger. Spike A did not get far enough to ask
   its own headline question.

The obvious explanation for (2) is `xdg-desktop-portal-gnome` showing a consent
dialog that no CI can click, leaving the D-Bus call outstanding. That is the
likely answer and it is **not yet proven** — a portal backend simply failing to
respond in a software-rendered session would look identical from the client
side. The next run captures the framebuffer at the moment of the keypress,
which is the one observation that separates "waiting for a human" from "broken":
if a dialog is on screen, it is in that frame.

If the dialog is confirmed, the question becomes whether the permission can be
pre-seeded in the image so the bind completes unattended — and that, not the
keypress, is what actually blocks Spike A. Whether Super+Space is *granted as
requested* also remains unanswered, because nothing was granted at all.

One incidental finding worth keeping: the trigger went out as `LOGO+space`, not
`SUPER+space`. `Trigger` renders the Super key with the XDG spec's modifier name
`LOGO`, which is correct — and it is why `triggers_look_equivalent` normalises
`logo`, `super`, `meta` and `win` onto one another. Had it not, a successful
bind would have been reported as a mismatch.

### What blocks Spike A is a dialog, and the dialog is avoidable

The previous section called the consent-dialog explanation "the likely answer,
not yet proven", because a portal backend simply failing to respond in a
software-rendered session looks identical from the client. It is proven now,
from the source of all three components rather than from the symptom, and the
same reading produced the way past it.

1. **`xdg-desktop-portal`'s frontend checks no permission.**
   `desktop-portal/global-shortcuts.c` includes `xdp-permissions.h` and then
   never calls it. There is no portal-level GlobalShortcuts permission, so
   there is nothing to pre-seed at the layer where one would look first.
2. **`xdg-desktop-portal-gnome` waits forever, on purpose.**
   `handle_bind_shortcuts` forwards straight to
   `org.gnome.Settings.GlobalShortcutsProvider` and completes only from the
   reply callback, on a proxy built with
   `g_dbus_proxy_set_default_timeout (settings, G_MAXINT)`. No timeout will
   ever fire, because a human may take arbitrarily long at a dialog. "No answer
   in 30 s" is the designed behaviour when nobody answers — not slowness, and
   not a fault. (The proxy also uses `G_DBUS_PROXY_FLAGS_NONE`, so Settings is
   D-Bus-activated on demand; "Settings was not running" was never it either.)
3. **`gnome-control-center` skips the dialog for an already-stored shortcut.**
   `cc_global_shortcut_dialog_present` opens with

   ```c
   if (!self->has_new_shortcuts) { emit_done (self, TRUE); return; }
   ```

   and `has_new_shortcuts` is set by `app_shortcuts_to_settings_variant`, which
   looks each requested shortcut up in what is already stored **by id alone**.
   Every id already present means nothing is new, which means no dialog — and
   that path still calls `store` and returns the full set, so `BindShortcuts`
   receives a proper reply rather than an empty one.

"Already stored" is `cc_keyboard_manager_get_global_shortcuts`, which is plain
GSettings on a relocatable schema:

| | |
|---|---|
| schema | `org.gnome.settings-daemon.global-shortcuts.application` |
| path | `/org/gnome/settings-daemon/global-shortcuts/<app_id>/` |
| key | `shortcuts`, type `a(sa{sv})` |

That is dconf, and dconf is image content. `packaging/vmtest/compass-shortcuts.dconf`
seeds the grant and the Containerfile compiles it into a **system** database:
`/var/home` is machine state on a bootc image, so a user database written at
build time is the kind of thing that works until it quietly does not. A system
db is not a lock, either — when the provider stores the grant it writes to the
user db, which shadows ours, exactly as it would for a human who clicked.

Two details that are easy to get wrong and cost nothing to get right:

- **The stored accelerator is not the portal's spelling.** The provider writes
  what `combo_get_accelerator` produces — GTK syntax, `<Super>space` — while
  the portal wire format for the same key is `LOGO+space`. The seed uses the
  former, because it is the shell that consumes it.
- **The id couples two files that are nowhere near each other.** The bypass
  keys off the id, so renaming `--id` without editing the dconf seed
  reinstates the dialog, and the resulting hang reads as a portal regression
  rather than as a typo. A unit test in `crates/vicinae/src/cli.rs` asserts the
  CLI default appears in the seed file, so that rename fails in tier 1 instead
  of thirty minutes into a VM run.

**This does not prove the keypress arrives**, and the pre-seed is not the
answer to Spike A — it is what finally lets the question be asked. Two things
are still open and the job now gathers evidence for both before the spike runs
(`checks.sh spike-a-evidence`): whether the seed is visible to the session at
all, and whether something else already owns Super+Space. GNOME binds the
input-source switcher to it by default, and a collision would present as
`activated: false` — indistinguishable, from inside the spike, from a portal
that does not deliver. Those are different answers and the run should not have
to guess between them.

### The pre-seed worked, and the third question finally got asked

First run with the seeded grant in the image:

```json
{
  "portal": "available (interface v1)",
  "requested_trigger": "LOGO+space",
  "bind_outcome": "granted",
  "bound": [{ "id": "compass.spike.toggle",
              "trigger_description": "Press <Super>space" }],
  "activated": false,
  "waited_seconds": 120.0
}
```

**`bind_outcome: granted`**, where every previous run said *the portal did not
answer `BindShortcuts` within 30 s*. The dialog was skipped, the bind completed
unattended, and a live binding for Super+Space existed in the session. The
reading of gnome-control-center's source was right and the seed does what it
was designed to do.

So for the first time there was a binding for the injected key to trigger, and
Spike A's headline question got asked rather than dodged. The answer is
**`activated: false`** — and that is a real result now, not the "unknown" of
every previous run.

**The obvious explanation is ruled out.** `checks.sh spike-a-evidence`, which
exists precisely for this, reported what else claims the chord:

```
switch-input-source          ['<Shift><Super>space']
switch-input-source-backward ['']
toggle-overview              @as []
```

GNOME on this image binds the input-source switcher to **Shift**+Super+Space
and leaves plain Super+Space alone. There is no collision. The hypothesis this
ADR recorded a section ago — that GNOME's own binding would eat the key — is
wrong, and the evidence step is what proved it wrong rather than leaving it as
a plausible story.

Two explanations remain, and they are indistinguishable in the report:

1. QEMU's injected key never reaches the Wayland session at all.
2. It reaches it, and the compositor does not route the *grabbed* shortcut
   through to the portal client.

`scripts/vmtest/spike-a.sh` now presses **Super alone** before the real
keypress, and screenshots either side. On GNOME that opens the Activities
overview, which is an unmissable change to the framebuffer: if the frame
changes, injection works and (2) is the answer; if it does not, (1) is, and no
amount of portal work would have helped. This ADR's own "what would change our
mind" already listed *"if corral's QMP key injection cannot produce Super+Space
in practice — expected to work but not run yet"*. It has now been run once, and
the control is what will say which half was at fault.

One incidental fix the run paid for. `trigger_matches_request` reported **false**
on a bind that granted exactly what was asked. GNOME does not return an
accelerator, it returns a sentence: `"Press <Super>space"`. The normaliser split
on `+`, `-` and space, so it compared `["press", "<super>space"]` against
`["space", "super"]`. It now strips the lead-in and treats the angle brackets of
GTK accelerator syntax as separators, with tests for both the sentence form and
a genuine mismatch dressed in the same syntax — the second half mattering more
than the first, since the easy fix here is one that turns a false negative into
a false positive and reports every trigger as correct.

## Spike B's first answer

Measured in the `Flatpak / build` job — a real bubblewrap sandbox on a hosted
runner, which is the environment the question is actually about:

```
inside a Flatpak:      yes
kernel:                6.17.0-1022-azure
Landlock (asked for): V1   ruleset: fully enforced
  reads inside allow:  yes (control)    reads outside deny: yes (assertion)
seccomp filter:        installed
  blocked call denied: yes (assertion)  other calls allowed: yes (control)
verdict: both confine a process here; Phase 4's sandbox design stands
```

**Both nest.** Landlock applies inside bubblewrap's sandbox and the kernel
reports the ruleset *fully* enforced — not the partial enforcement a kernel
older than the requested ABI would give. seccomp installs a second filter on
top of bubblewrap's own and it bites. In both cases the control passed too, so
this is a boundary that denies what it should while still allowing what it
should, rather than one that denies everything or nothing.

This is what #7 was waiting on. Phase 4 may design its extension host on
Landlock for the filesystem boundary and seccomp for the syscall filter, on this
kernel class, inside the Flatpak we ship. The risk PLAN.md §6 flagged is
retired.

Three caveats, none of which change the verdict:

- ~~**This is one kernel, not the target's.**~~ **Settled by the VM run.** The
  first measurement was on the hosted runner's `6.17.0-1022-azure`, and since
  Landlock's ABI is a kernel property that said nothing about the platform we
  ship to. The compass VM job has now run the same spike on **Bluefin's own
  `7.1.8-200.fc44.x86_64`**, inside the real Flatpak, in a real GNOME session,
  and returned the same verdict with the same four rows green:

  ```
  inside a Flatpak:      yes
  kernel:                7.1.8-200.fc44.x86_64
  Landlock (asked for): V1
    ruleset:             fully enforced
    reads inside allow:  yes           (control: must stay yes)
    reads outside deny:  yes           (the assertion)
  seccomp filter:        installed
    blocked call denied: yes           (the assertion)
    other calls allowed: yes           (the control)
  ```

  Two kernels, two Flatpak sandboxes, one answer. This is the row that lets
  Phase 4 be designed rather than guessed at.
- **`seccomp_mode` came back `null` in the JSON**, despite the filter
  demonstrably working. (The VM check runs the human-readable form, which does
  not carry the field, so the VM run neither confirms nor contradicts this.) The field is read from `/proc/self/status`, which the
  Flatpak sandbox evidently does not expose the way an unsandboxed process sees
  it. So the kernel's own account of the filter — the field that caught the
  `SYS_mkdir`/`SYS_mkdirat` bug during development — is *unavailable in exactly
  the environment we care most about*. The assertion and control still stand on
  their own, and the verdict rests on them; but the corroborating evidence is
  absent here, and a future failure in this job will be harder to diagnose
  because of it.
- **V1 is asked for, never detected**, per the `landlock` crate's own guidance
  that runtime detection makes sandboxing non-deterministic. A kernel offering
  more gives us no more. That is deliberate.

## The tier finally points at the product

Everything above tests the platform: does Bluefin boot, does GDM autologin, is
there a Wayland session, a session bus, a portal, a sandbox that nests. All of
it was necessary and none of it is the launcher, because until #29 there was no
launcher to open — `compass-ui` was a library and nothing started a window.

The `launcher` job opens one. It is a third VM job rather than a check inside
`boot-compass`, for the same structural reason Spike A is: the only observer of
the framebuffer is corral, on the host side of QEMU, and corral runs each
`--check` over its own SSH connection with no way to interleave a host command.
So the run leaves the VM up and `scripts/vmtest/launcher.sh` drives guest, host,
guest — start the launcher over SSH, screenshot and type from the host, then ask
the guest whether it is still alive.

**What is gated is narrower than what is recorded, deliberately.** Gated: the
launcher process starts and stays up, and the screen still passes corral's own
blank test with the launcher open. Recorded but not gated: the three luminance
deviations, before, open, and after typing.

It is tempting to assert that opening a launcher raises the deviation, and it
almost certainly does. But that has never been measured once, and this ADR
already says that inventing a pixel threshold is how a tier starts flaking —
the same reasoning that keeps `--require-paint` at corral's 0.02 rather than at
a number we chose. Both spikes shipped gating on nothing but "produced a
report", and both were more useful for it. Once a few runs have published
numbers, the gate can be set from data; that is a two-line change to the driver.

Two details carried over from earlier mistakes in this tier, because both cost a
run to learn:

- **The readiness predicate matches the process *name*.** The obvious
  `pgrep -f` against something distinctive also matches the shell evaluating it,
  so the predicate is true before the launcher has done anything — the same
  reflexivity that made Spike A's collector time out 180 s every run. Verified
  both directions locally: no match with nothing running, a match with a real
  process of that name.
- **"The process is alive" is not "a window is on screen"**, and nothing inside
  the guest can tell the difference. That is why the host screenshot exists, and
  why neither half is sufficient alone. The before-frame is the control: without
  it, "the launcher drew" cannot be told from "the desktop always looked like
  that".

There is a side benefit worth stating, since it addresses a risk recorded above.
The compass image passes `--require-paint` at **0.0254** against a 0.02
threshold, because a GNOME desktop under llvmpipe is mostly flat dark pixels,
and that margin is thin enough to flap. A launcher on screen is the honest way
to widen it. The dishonest way is to move the line, which this ADR has already
ruled out.

## What would change our mind

- If corral's QMP key injection cannot produce Super+Space in practice — `meta_l` is passed through
  to QEMU unvalidated, so this is expected to work but has **not been run yet** — the hotkey half of
  Spike A needs another mechanism, though the portal-binding half still works over SSH.
- If hosted runners lose KVM, the tier moves to a self-hosted runner rather than being abandoned;
  the harness is unchanged either way.
