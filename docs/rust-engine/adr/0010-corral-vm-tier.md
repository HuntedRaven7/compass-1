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

## Two answers from the first run of the launcher job, and neither is comfortable

The launcher job and Spike A's Super-alone control ran together for the first
time. Both produced clear results, and both are things the tier existed to
find. Neither was visible from any other tier.

### 1. QMP key injection does not reach this GNOME session

The control pressed **Super alone**, which on GNOME opens the Activities
overview — an enormous, unmissable change. The two frames either side are
**byte-identical**. Not "similar", not "the deviation was unchanged": the same
bytes.

That is conclusive, and it is conclusive *because the capture is demonstrably
live*. In the launcher job on the same commit, three frames from the same
mechanism differ from one another at the pixel level. So the screenshots are
real and current, and the overview genuinely did not open.

This ADR's "what would change our mind" already contained the line:

> If corral's QMP key injection cannot produce Super+Space in practice —
> `meta_l` is passed through to QEMU unvalidated, so this is expected to work
> but has **not been run yet** — the hotkey half of Spike A needs another
> mechanism.

It has now been run. **It does not work, and the hotkey half of Spike A needs
another mechanism.** Spike A's `activated: false` is explained: the key never
arrived. Not the portal, not the compositor's routing, not our event loop —
which the `Changed` signal arriving through that same broadcast channel during
the wait had already made unlikely.

The first hypothesis was that corral adds **no input device** — its command
line is `-vga virtio -display none` with `virtio-net-pci` and `virtio-rng-pci`
and nothing else. That was hedged as a guess that had earned a measurement, and
the measurement refuted it. `/proc/bus/input/devices` in the guest:

```
N: Name="AT Translated Set 2 keyboard"
H: Handlers=sysrq kbd leds event1
B: EV=120013
```

x86's default machine type provides the PS/2 controller, the kernel binds it,
and there is a working evdev node. **There is a keyboard.**

So two possibilities remain, and they are indistinguishable from outside the
guest: QEMU never delivers the scancode to that emulated keyboard, or it does
and something above the kernel — mutter, the seat, focus — discards it.

Reading the evdev node decides it, and needs nothing installed.
`checks.sh keyboard-capture-start` opens `/dev/input/eventN` for the duration of
the keypress and `keyboard-capture-report` says how many bytes arrived. Bytes
mean the kernel saw the event and the loss is above it; silence means QEMU never
delivered it, and it is corral's to fix rather than ours. The node is resolved
from `/proc/bus/input/devices` rather than hard-coded to `event1`, because a
hard-coded node that silently moves would report "no input" for a keyboard that
is working perfectly — and the resolver was tested against the guest's actual
output before shipping.

### 2. The launcher starts, stays alive, says nothing, and draws nothing

`vicinae ui` reached a running process in 2 s and was still running at the end.
It printed **not one line**. And its window never appeared.

That last part took real measurement rather than a glance, and the glance would
have been wrong twice over. All five screenshots across both jobs reported a
luminance deviation of **0.1564**, identical to four decimal places, which
looks exactly like a frozen framebuffer. It is not: the PNGs have different
checksums. Decoding them and counting pixels gives the real picture —
before→open differs by 1.60%, in a single box **258 × 81 at x 498–755,
y 716–796**.

The launcher's window is configured 640 × 480, centred, which on this 1280 × 800
display is x 320–960, y 160–640. The region that changed is the wrong size and
in the wrong place — bottom-centre, where GNOME draws OSDs and notifications.
**Whatever appeared, it was not our window.**

Two lessons, one about the product and one about this tier:

- A global luminance deviation is far too coarse to answer "did a window
  appear". Three visibly different frames shared it to 4 dp. Anything this tier
  wants to claim about *what* is on screen has to come from pixels, not from the
  summary statistic — and the artifacts are uploaded precisely so that can be
  done after the fact, which is how this was.
- **This vindicates not gating on the deviation.** The temptation, written down
  and resisted one section ago, was to assert that opening a launcher raises it.
  Had that gate existed it would have been *green* here — 0.1564 is comfortably
  above the 0.02 blank threshold and unchanged — while the launcher drew
  nothing at all. A gate that passes on the exact failure it was meant to catch
  is worse than no gate, because it is also a claim.

The next run is instrumented rather than guessed at: `RUST_LOG` now carries
`wgpu`, `wgpu_hal`, `iced_wgpu` and `winit` at debug, with `RUST_BACKTRACE=1`.
`WGPU_BACKEND` is deliberately *not* pinned — naming a backend would decide the
answer instead of measuring it, and what is wanted is wgpu's own account of
which adapters exist under llvmpipe. The most likely story is that Iced cannot
get a rendering surface in a VM with no GPU, but that is a hypothesis with a
silent process behind it, which is the least trustworthy kind.

Neither finding is gated, and neither should be yet. A red X saying "the
launcher does not draw" every run, before anyone knows why, trains people to
ignore the tier. Both are recorded, both are instrumented, and the gate follows
the diagnosis rather than preceding it.

### Instrumented: it never reaches wgpu, and my hypothesis was wrong

The previous section guessed that Iced could not get a rendering surface in a
VM with no GPU, and flagged that as a hypothesis with a silent process behind
it. The instrumented run says it is wrong.

With `wgpu`, `wgpu_hal`, `iced_wgpu` and `winit` all at debug, the launcher
produced exactly three log records and then went quiet:

```
INFO  iced_winit: System theme: None
INFO  iced_winit: Window attributes for id `Id(1)`: WindowAttributes { … }
ERROR winit::ActiveEventLoop::create_window{…}: sctk_adwaita::config:
      XDG Settings Portal did not return response in time:
      timeout: 100ms, key: color-scheme
```

The last is at **T+200 ms**. The log read again at the *end* of the run — after
both screenshots and the typed query, ten seconds later — is byte-identical.

**There is not one wgpu line.** Not an adapter enumeration, not a failure, not
a warning. So execution never reaches wgpu initialisation at all, and "wgpu
cannot find an adapter under llvmpipe" is not the explanation. The failure is
earlier, inside or just after `create_window`.

The one error on the way is `sctk-adwaita` — the client-side decoration
provider — timing out after 100 ms reading `color-scheme` from the XDG Settings
portal. That timeout is **not** the hang, and this is settled from source rather than
from another run. `sctk-adwaita 0.10.1`'s `config.rs` implements both of its
portal queries by shelling out to `dbus-send --reply-timeout=100` and taking
`.output()`. Both are bounded at 100 ms; neither can block. The logging is
self-consistent too: the error only fires when `dbus-send` actually ran and
returned empty stdout, so the binary exists in the runtime and simply timed
out, and `prefer_dark()` then returns `false` and carries on.

So the most visible error in the log is a red herring, and worth naming as one
before somebody spends a day on the portal. What remains is that the process
blocks somewhere after that and before wgpu — inside or just after
`create_window` — with nothing logging on the way.

Logs cannot settle it, because the code that is stuck is not the code that is
logging. The kernel can. `checks.sh launcher-diagnose` reads
`/proc/PID/{status,wchan,syscall}` per thread and resolves the process's open
sockets, which names the syscall a thread is parked in and what it is parked
on — a Wayland socket and a D-Bus socket look nothing alike. No debugger, no
package to install, and it works on a stripped image. Reading the host's
`/proc` for a Flatpak process is fine: bwrap namespaces the guest's view of
`/proc`, not root's.

It runs **twice**, before and after the keystroke, deliberately. A thread
parked in the same syscall on the same socket both times is stuck; one that has
moved is merely slow, and those want completely different fixes.

`RUST_LOG` also gains `sctk_adwaita`, `smithay_client_toolkit`, `wayland_client`
and `calloop` at debug, so the Wayland protocol exchange is visible next time
rather than inferred.

Worth stating plainly, because it is the second time in two sections: the
obvious explanation was wrong again. First the input-source collision that
turned out to be `<Shift><Super>space`, now wgpu that turns out never to run.
Both were caught by building the measurement before believing the story.

### What the kernel said, and what it does not prove

The probe ran, twice, and the two readings are **byte-identical** — same thread,
same `wchan`, same syscall, same stack pointer, same arguments:

```
pid 4212   State: S (sleeping)   Threads: 13
  tid 4212  wchan=poll_schedule_timeout   syscall: 7  (poll)  nfds=2  timeout=0xffffffff
  tid 4225  wchan=ep_poll                 syscall: 281 (epoll_wait)
  … eleven more, all futex_do_wait
```

So the main thread is parked in `poll()` on **two** file descriptors with an
**infinite** timeout, and nothing moved between before-the-keystroke and
after-it.

**The tempting reading is wrong, and worth saying so before anyone repeats it.**
"Blocked in an infinite poll" sounds like a deadlock, but a two-fd infinite poll
is exactly what winit's Wayland event loop looks like when it is *idle* — the
display fd plus calloop's eventfd. A perfectly healthy launcher sitting with
nothing to do would show the same three lines. Identical stack pointers across
probes prove only that it has not moved, which an idle event loop also has not.

What makes it a fault is the combination with what is missing:

| evidence | says |
|---|---|
| main thread in winit's event-loop poll | `create_window` returned; the loop is running |
| **no wgpu log records at all** | `Compositor::new` never ran to the point of touching wgpu |
| window never became visible | `iced_winit` only calls `set_visible(true)` after `window.renderer` exists |

Reading `iced_winit 0.14.0` closes the loop: compositor creation happens inside
`runtime.block_on(create_compositor)`, and the window is shown only once a
renderer exists. So the launcher is not wedged in a syscall it cannot leave — it
is **waiting for a Wayland event that never arrives**, with the window created,
hidden, and no renderer behind it.

That is as far as the evidence goes. What it does *not* establish is why the
compositor never sends that event, and the honest list of candidates is still
open: a surface configure that mutter withholds under llvmpipe, something about
this window's own attributes (`transparent: true` with `decorations: false` is
an unusual pair), or a winit/iced interaction specific to a software-rendered
session. Picking one now would be the fourth guess in a row on this bug, and
the previous three were all wrong.

One diagnostic lesson, paid for immediately. The probe's socket step printed
`(ss unavailable or no match)` — one message for two conditions that want
completely different fixes, so it said nothing useful about either. It now
distinguishes them, and falls back to matching fd inodes against
`/proc/net/unix`, which needs no tools at all. That fallback's limit is stated
in the code rather than discovered in the guest: a *connected* AF_UNIX socket
has an empty path column, so it names listening sockets and leaves client ends
unnamed. Checked on this machine before shipping.

### The control this job should have had from the start

Every measurement so far says our launcher puts no window on screen. **None of
them distinguishes that from nothing being able to.** A session in which no
client can render at all would produce byte-for-byte the same evidence, and
would exonerate the launcher completely.

The desktop painting does not settle it. Deviation 0.1564 is GNOME Shell
compositing its own furniture — a top bar, a wallpaper — not a client surface.
No client application has been shown to draw in this session, ever, by any job
in this tier.

So `checks.sh control-app-start` launches a stock GNOME application and the
host screenshots it. If it draws and ours does not, the fault is ours. If
neither draws, the fault is the session and every conclusion about the launcher
above is void. This is the same assertion-plus-control shape as Spike B and the
Super-alone keypress, and it should have been here in the first version of the
job rather than three runs later.

The application is picked at runtime from what the image actually has, rather
than hard-coded: a name that turns out to be absent reads as "the control
failed" when it means "the control never ran", and those are opposite
conclusions.

**A note on the predicate, because this is the fourth time.** The obvious way to
wait for that application is `pgrep -f "$app"`, and it is wrong for the same
reason it was wrong in Spike A's collector: the app's name is in the command
line of the shell doing the matching, so the predicate matches itself and is
true immediately. `pgrep -x` is not the escape either — `comm` is truncated to
15 characters, so `gnome-text-editor` is `gnome-text-edit` and an exact match
silently never fires. The check compares the resolved `/proc/PID/exe` instead,
which is neither a string anyone typed nor truncated. Tested three ways before
shipping: absent binary, running binary, and the sentinel short-circuit.

That failure mode has now appeared four times in this tier — `pgrep -f` in the
collector, `ls /proc/*/fd | grep wayland-` in the first launcher predicate,
`pgrep -c` in a throwaway wait loop, and `pgrep -f` again here. Three were
caught by testing the predicate before trusting it. The one that was not cost
seventy minutes of a loop that could never terminate. **A predicate that has
not been run against both a true and a false case is not a predicate, it is a
guess** — which is the same rule this ADR already applies to sandboxes and
paint gates, arrived at from a different direction.

## Retraction: the launcher draws. It was measured too early, twice.

Three sections above say the launcher does not put a window on screen. **That
is wrong, and the control added to catch exactly this kind of mistake is what
caught it.**

The run that added a stock GNOME application as a control also, incidentally,
delayed the screenshot by the few seconds the diagnostic step takes. In that
run the launcher's own window is plainly there:

| | pixels changed | bounding box |
|---|--:|---|
| our launcher | 8.22% | x 335–942, y 152–796 |
| nautilus (control) | 16.80% | x 166–1122, y 206–796 |

The launcher's window is configured **640 × 480 centred**, which on this
1280 × 800 display is x 320–960, y 160–640. The measured box is x 335–942,
starting at y 152. That is our window.

And the log carries **119 wgpu records** where the previous run had none:

```
wgpu_core::instance: Instance::new: created Vulkan backend
wgpu_core::instance: Adapter AdapterInfo { name: "llvmpipe (LLVM 19.1.7, 256 bits)",
                     device_type: Cpu, driver: "llvmpipe",
                     driver_info: "Mesa 26.1.8", backend: Vulkan }
```

Software rendering works. Vulkan through lavapipe, Mesa 26.1.8. Everything the
earlier sections treated as broken is fine.

### What actually happened

Startup under llvmpipe is slow and the spread is wide. Timestamps from the two
runs, same image, same job:

| | launcher started | wgpu initialising | window seen |
|---|---|---|---|
| earlier run | 13:23:47.0 | *not yet at 13:23:55.1* (8.1 s in) | no |
| later run | 14:21:16.6 | 14:21:18.9 (2.4 s in) | 14:21:20.3 (3.7 s) |

The job screenshotted as soon as the *process* existed. In the fast run that
happened to be late enough; in the slow ones it was not. `launcher-start` was
waiting on the wrong thing.

### The rule I broke is this ADR's own

"Key assertions off states, never off durations" is written near the top of this
document, learned from `graphical.target` and from GDM blanking the framebuffer.
Then `launcher-start` waited for a process to exist — which is a state, but not
the state that matters — and everything downstream sampled a moment.

`launcher-start` now waits for `Adapter AdapterInfo` in the launcher's own log:
wgpu reports a chosen adapter only once it has a surface to render to, which is
a real state and a much later one. The three-second settle after it is slack
after a state, the same shape and justification as `wait-graphical.sh`.

### What survives, and what does not

**Does not survive:** "the launcher does not draw", the 258 × 81 bottom-centre
box read as "GNOME's furniture, not ours" (it was the launcher not yet painted,
plus a notification), and PLAN §12's item 0 as originally written.

**Survives, and is still true:** QMP key injection does not reach this session
(the Super-alone control frames are byte-identical, and that is independent of
any of this); the deviation statistic is too coarse to answer "did a window
appear" — 0.1564 to four decimal places across visibly different frames — and
gating on it would still have been wrong, though for the opposite reason than
the one recorded earlier; and the `sctk-adwaita` portal timeout is still a red
herring, still bounded, still not the cause of anything.

The habit that produced the error is worth naming as precisely as the error. I
built a control for the *keypress* question and for the *sandbox* question, and
did not build one for "is anything on screen" until three runs in — and the
moment it existed it overturned the conclusion. A measurement with no control is
a guess with a number attached, and it took a stock file manager to say so.

### The first gate in this tier taken from a measurement

Every earlier section refused to gate on the framebuffer, and was right to: a
threshold invented before anyone had seen a number is how a tier starts flaking,
and the deviation gate that was nearly added would have passed on the exact
failure it was meant to catch.

There is now a number, and it is reproducible. Two consecutive runs agreed **to
the pixel**: 84150 changed (8.22%) in a box at x 335–942, y 152–796, against a
window configured 640 × 480 centred — x 320–960, y 160–640.

So `scripts/vmtest/framediff.py` decodes both PNGs and counts, and the job
asserts that opening the launcher changes **at least 3%** of the screen **inside
x 300–980, y 140–800**. Both bounds are deliberately loose around the
measurement: the gate exists to catch "nothing was drawn", not to pin the exact
pixels of a theme. A tight bound would break on the first font change and teach
everyone to ignore the job — which is the same failure mode as a permanently red
X, reached by the opposite route.

`framediff.py` decodes PNG with `zlib` and an unfilter loop rather than Pillow,
which is not on the runner and is not worth a dependency here. It reports by
default and only asserts when asked, so it stays usable for the next question
before there is a number to gate on. It was tested against the real artifacts in
all five shapes before shipping: report-only, an assertion that must pass, a
threshold that must fail, a box that must fail, and identical frames.

It is also now covered by tier 1. `shell.yaml` compiled *embedded* Python out of
the shell scripts and nothing else, so a standalone `.py` would have had its
first syntax check thirty minutes into a VM run — the precise thing that
workflow exists to prevent. It compiles both now.

### The typing step is labelled, not deleted

`corral type` cannot currently reach this session, since QMP key injection does
not arrive at all. The step stays, with that stated in the script: deleting it
would lose a free regression check — the day injection works, that frame changes
and says so — while leaving it unlabelled is worse than either, because it reads
as a test of the launcher's input handling and is not one.

## What would change our mind

- If corral's QMP key injection cannot produce Super+Space in practice — `meta_l` is passed through
  to QEMU unvalidated, so this is expected to work but has **not been run yet** — the hotkey half of
  Spike A needs another mechanism, though the portal-binding half still works over SSH.
- If hosted runners lose KVM, the tier moves to a self-hosted runner rather than being abandoned;
  the harness is unchanged either way.
