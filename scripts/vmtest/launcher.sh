#!/usr/bin/env bash
# Drive the launcher: open it in the guest, look at the screen, type at it.
#
# Usage: scripts/vmtest/launcher.sh <artifact-dir> <vm-name>
#
# WHY THIS IS NOT A `--check`
#
# Same reason as spike-a.sh. The question here is "did a launcher appear on
# screen", and the only observer of the framebuffer is corral, on the host side
# of QEMU. Nothing inside the guest can answer it — ADR-0010 says so and it is
# still true. So the sequence straddles guest and host, which means running
# after `corral vmtest` returns, against a VM it was told to leave up.
#
# WHAT IS GATED, AND WHAT IS ONLY RECORDED
#
# Gated: the launcher process starts and stays up (checks.sh, in the guest),
# and the screen still passes corral's own blank test with the launcher open.
# That threshold is corral's, not one invented here.
#
# Recorded, not gated: the three luminance deviations. It is tempting to assert
# that opening a launcher raises the deviation, and it probably does — but this
# has never been measured once, and ADR-0010 is explicit that inventing a pixel
# threshold before seeing real numbers is how a tier starts flaking. Spike A and
# Spike B both shipped gating on nothing but "produced a report", for the same
# reason. Once a few runs have published numbers, the gate can be set from data
# rather than from a guess, and that is a two-line change to this file.
set -euo pipefail

out="${1:?usage: launcher.sh <artifact-dir> <vm-name>}"
vm="${2:?usage: launcher.sh <artifact-dir> <vm-name>}"
query="${LAUNCHER_QUERY:-fi}"
checks=/usr/libexec/compass-vmtest/checks.sh

# EVERY CALL THAT LEAVES THIS SCRIPT HAS A BUDGET. (#333)
#
# The job has one timeout, seventy-five minutes, and until now it was the only
# bound on anything here. An SSH connection to a guest that stopped answering,
# a checks.sh subcommand stuck behind a dead session bus, a corral screenshot
# waiting on a wedged QMP socket -- each of those sat until the job was killed,
# and a job killed by its own timeout uploads no explanation. Worse, the
# diagnostic collector below runs exactly when the guest is least likely to
# answer, and it could spend the rest of the job's time asking.
#
# So each kind of call gets its own budget, sized from what the guest side is
# known to wait for internally rather than guessed:
#
#   VMTEST_SSH_CONNECT_S      15   reaching sshd at all (ssh ConnectTimeout,
#                                  which also bounds the banner exchange)
#   VMTEST_GUEST_BUDGET_S    300   one checks.sh subcommand; launcher-start
#                                  alone waits up to 90 + 150 s inside the guest
#   VMTEST_PROBE_BUDGET_S     20   one read-only observation (ui-state, status)
#   VMTEST_CORRAL_BUDGET_S    60   one corral screenshot, type or key
#   VMTEST_DIAGNOSTICS_BUDGET_S 60 the journal collected after a failure
#   VMTEST_SETTLE_BUDGET_S    30   one hide/summon/panel transition, all attempts,
#                                  and one wait for a still desktop frame
#   VMTEST_SESSION_BUDGET_S  120   GNOME Shell finishing its startup after the
#                                  boot marker (4 s after it in run 42074769175)
#   VMTEST_STABLE_SHOTS        3   consecutive alike frames that make "still"
#   VMTEST_POLL_S              1   the pause between two attempts at one
#   VMTEST_KILL_GRACE_S        5   SIGTERM to SIGKILL, for a call over budget
#
# Overridable so the negative tests (launcher-selftest.py) can hang a stub and
# watch it be stopped in seconds rather than minutes; the defaults are what CI
# runs.
ssh_connect_s="${VMTEST_SSH_CONNECT_S:-15}"
guest_budget_s="${VMTEST_GUEST_BUDGET_S:-300}"
probe_budget_s="${VMTEST_PROBE_BUDGET_S:-20}"
corral_budget_s="${VMTEST_CORRAL_BUDGET_S:-60}"
diagnostics_budget_s="${VMTEST_DIAGNOSTICS_BUDGET_S:-60}"
settle_budget_s="${VMTEST_SETTLE_BUDGET_S:-30}"
session_budget_s="${VMTEST_SESSION_BUDGET_S:-120}"
stable_shots="${VMTEST_STABLE_SHOTS:-3}"
poll_s="${VMTEST_POLL_S:-1}"
kill_grace_s="${VMTEST_KILL_GRACE_S:-5}"

# corral wrote result.json as root and it holds the run's own SSH details.
# Reading them beats rebuilding the invocation here and drifting out of step.
#
# ConnectTimeout bounds reaching sshd; ServerAlive* notice a guest that went
# away under an established connection (three unanswered probes five seconds
# apart), which no budget on the remote command can tell apart from a command
# that is merely slow.
sudo chown -R "$(id -un)" "$out" 2>/dev/null || true
mapfile -t ssh_argv < <(RESULT="$out/result.json" CONNECT_S="$ssh_connect_s" python3 - <<'PY'
import json, os
ssh = json.load(open(os.environ['RESULT']))['ssh']
for part in ['ssh',
             '-i', ssh['identityFile'],
             '-p', str(ssh['port']),
             '-o', 'StrictHostKeyChecking=no',
             '-o', 'UserKnownHostsFile=/dev/null',
             '-o', 'LogLevel=ERROR',
             '-o', 'BatchMode=yes',
             '-o', f"ConnectTimeout={os.environ['CONNECT_S']}",
             '-o', 'ServerAliveInterval=5',
             '-o', 'ServerAliveCountMax=3',
             f"{ssh['user']}@{ssh['host']}"]:
    print(part)
PY
)

# The bounded call in flight, so a cancellation can stop it rather than leave
# it running after this script is gone.
#
# `timeout` puts itself and the command in a process group of their own, which
# is what lets it stop a command's descendants as well as the command. The
# price is that a Ctrl+C or a runner's SIGTERM sent to THIS script's group no
# longer reaches them -- so the call runs in the background and is waited for,
# which lets the traps below fire at once and hand the signal on. `timeout`
# forwards a signal it receives to its whole group, so passing it the signal is
# enough to reap everything the call started and nothing it did not.
active_pid=""
cancelled_by=""
run_bounded() {
  local status=0
  "$@" &
  active_pid=$!
  wait "$active_pid" || status=$?
  active_pid=""
  return "$status"
}

cancel() {
  cancelled_by="SIG$1"
  if [ -n "$active_pid" ]; then
    kill -TERM "$active_pid" 2>/dev/null || true
    wait "$active_pid" 2>/dev/null || true
  fi
  echo "CANCELLED: the run received $cancelled_by; the call in flight was stopped." >&2
  exit "$2"
}
trap 'cancel INT 130' INT
trap 'cancel TERM 143' TERM

# guest_within BUDGET COMMAND...: run COMMAND in the guest, stopped at BUDGET.
#
# The command's own status is returned untouched, so every gate still fails
# with its own verdict. Only the two statuses that are not a verdict are named,
# because "exit 124" and "exit 255" in a log say nothing to the next reader:
# 124/137 are `timeout` stopping it, 255 is ssh failing to reach the guest or
# losing it.
#
# stdin is /dev/null because nothing here feeds the guest input, and an ssh
# left reading a terminal it does not own would stop on SIGTTIN and sit there
# until its budget ran out.
guest_within() {
  local budget="$1" status=0
  shift
  run_bounded timeout --kill-after="$kill_grace_s" "$budget" \
    "${ssh_argv[@]}" "$@" </dev/null || status=$?
  case "$status" in
    124|137)
      echo "TIMEOUT: the guest command \`$*\` did not finish within its ${budget}s budget and was stopped." >&2 ;;
    255)
      echo "SSH: the guest did not answer \`$*\`: ssh exited 255, so no connection was made within ${ssh_connect_s}s or it was lost." >&2 ;;
  esac
  return "$status"
}
guest() { guest_within "$guest_budget_s" "$@"; }

# `sudo -E "$(command -v corral)"`, not `sudo corral`: corral is on PATH via
# GITHUB_PATH, and sudo replaces PATH with secure_path, so `sudo corral` is
# "command not found". Verified that -E alone does not help — secure_path wins.
#
# `timeout` runs under sudo, not around it: corral is root's process, and a
# `timeout` running as this user could signal sudo but not reap what corral
# leaves behind when SIGKILL is needed. As root it can.
corral_bin="$(command -v corral)"
host_corral_within() {
  local budget="$1" status=0
  shift
  run_bounded sudo -E timeout --kill-after="$kill_grace_s" "$budget" \
    "$corral_bin" "$@" || status=$?
  case "$status" in
    124|137)
      echo "TIMEOUT: \`corral $1\` did not finish within its ${budget}s budget and was stopped." >&2 ;;
  esac
  return "$status"
}
host_corral() { host_corral_within "$corral_budget_s" "$@"; }
# shot FRAME [CORRAL-FLAG...]
shot() { host_corral screenshot "$vm" -o "$out/$1" "${@:2}"; }

# WHEN A GATE FAILS, TAKE THE GUEST'S JOURNAL WITH US.
#
# Every gate below aborts the script under `set -e`, and until now that is all
# it did: the artifact held screenshots and a framediff line, and anything
# systemd knew about why -- a portal that exited, a unit that never started, a
# session that came up wrong -- was gone with the VM thirty seconds later. The
# next run costs twenty-five minutes, so the evidence has to be collected on
# the way out rather than asked for afterwards.
#
# An EXIT trap rather than ERR: ERR does not fire for a failure inside a
# function or a compound, and most of this file is both. The status is checked
# so that a passing run does not pay for a journal nobody will read.
#
# `|| true` on the collection itself, twice over. A guest that has stopped
# answering SSH is exactly when this runs, and a diagnostic that fails must not
# replace the real exit status with its own -- that would turn a specific gate
# failure into an unexplained one, which is the opposite of the point.
#
# And bounded, because "the guest has stopped answering" is the likeliest
# reason to be here: an unbounded collector would spend the rest of the job
# waiting on the one machine that is known to be stuck. When collection fails
# it says so on a line of its own, marked SECONDARY, so the reader can see both
# that the evidence is missing and that its absence is not why the run failed.
#
# A cancelled run collects nothing: the person or runner that cancelled it is
# not waiting for a journal, and a minute spent fetching one is a minute of a
# cancellation not taking effect.
collect_diagnostics() {
  local status=$? diag_status=0
  trap - EXIT
  if [ "$status" -eq 0 ]; then
    return 0
  fi
  if [ -n "$cancelled_by" ]; then
    echo "=== the run was cancelled by $cancelled_by (status $status): no diagnostics collected ==="
    return "$status"
  fi
  echo
  echo "=== the run failed (status $status): collecting the guest's journal ==="
  guest_within "$diagnostics_budget_s" "$checks" journal > "$out/journal.txt" 2>&1 \
    || diag_status=$?
  tail -n 40 "$out/journal.txt" || true
  if [ "$diag_status" -ne 0 ]; then
    echo "SECONDARY: collecting the journal failed with status $diag_status; what it printed is in journal.txt."
    echo "  That is a second problem, not the reason this run failed. The run's own status is $status, and it is the status this job exits with."
  else
    echo "--- the full journal is journal.txt in this job's artifact ---"
  fi
  return "$status"
}
trap collect_diagnostics EXIT

# WAITING FOR A TRANSITION, NOT SLEEPING THROUGH ONE. (#333)
#
# Hide, summon and the action panel each used to be followed by `sleep 5` and
# a screenshot. Five seconds was a guess that held on most runs: on a slow
# llvmpipe frame it was too short and the gate failed on a frame taken before
# the repaint, and on every other run it was five seconds spent doing nothing.
#
# What replaces it waits for two things, in order, because either alone can be
# satisfied by the wrong screen:
#
#   the launcher's own state line says the transition happened (ui-state), and
#   a screenshot shows the screen in the state that transition should produce,
#   judged against a named reference frame by framediff.
#
# Pixels that have stopped changing are never the condition. A frozen error
# dialog is perfectly stable, and so is a launcher that never went away.
#
# Only observations are retried -- ui-state, launcher-status and screenshots
# change nothing in the guest. The toggle, the summon and the chord that cause
# each transition run once, before the wait, and are never repeated: a second
# toggle would undo the first and could make a broken hide look like a slow
# one.
#
# Every attempt is kept under attempts/ in the artifact, numbered, with what it
# printed, so a failed wait shows the whole sequence it saw and not only the
# last frame.
attempts_dir="$out/attempts"

# The budget for one attempt: the probe budget, or whatever is left of the
# transition's budget if that is less, so one hung attempt cannot carry the
# wait past its deadline.
attempt_budget() {
  local deadline="$1" cap="$2" left
  left=$(( deadline - SECONDS ))
  if [ "$left" -lt 1 ]; then left=1; fi
  if [ "$left" -lt "$cap" ]; then echo "$left"; else echo "$cap"; fi
}

# await_state SLUG WHAT KEY=VALUE...
#
# Polls `checks.sh ui-state KEY=VALUE...` until the launcher's last state line
# carries every KEY=VALUE. Returns 0 when it does; 1 when the launcher process
# has exited, which no amount of waiting will change; 255 when the guest stops
# answering SSH, for the same reason; 124 when the budget runs out.
await_state() {
  local slug="$1" what="$2"
  shift 2
  local deadline=$(( SECONDS + settle_budget_s )) attempt=0 status log
  mkdir -p "$attempts_dir"
  while :; do
    attempt=$(( attempt + 1 ))
    log="$attempts_dir/$slug-state-$(printf '%02d' "$attempt").txt"
    status=0
    guest_within "$(attempt_budget "$deadline" "$probe_budget_s")" \
      "$checks" ui-state "$@" >"$log" 2>&1 || status=$?
    if [ "$status" -eq 0 ]; then
      echo "ok: $what (attempt $attempt)"
      sed -n '/^--- asserting/,$p' "$log"
      return 0
    fi
    if [ "$status" -eq 255 ]; then
      cat "$log"
      echo "FAIL: the guest stopped answering SSH while waiting for $what." >&2
      return 255
    fi
    # Not there yet, or the probe itself timed out. Which of those it was is in
    # the attempt's log; what decides whether to keep waiting is whether the
    # launcher is still alive to get there.
    if [ "$status" -ne 124 ] && [ "$status" -ne 137 ]; then
      status=0
      guest_within "$(attempt_budget "$deadline" "$probe_budget_s")" \
        "$checks" launcher-status >>"$log" 2>&1 || status=$?
      case "$status" in
        0|124|137) ;;
        255)
          tail -n 20 "$log"
          echo "FAIL: the guest stopped answering SSH while waiting for $what." >&2
          return 255 ;;
        *)
          tail -n 40 "$log"
          echo "FAIL: the launcher exited while waiting for $what." >&2
          return 1 ;;
      esac
    fi
    echo "  attempt $attempt: not yet ($(basename "$log"))"
    if [ "$SECONDS" -ge "$deadline" ]; then
      break
    fi
    sleep "$poll_s"
  done
  echo "--- the last attempt ---"
  cat "$log"
  echo "FAIL: TIMEOUT: $what did not happen within ${settle_budget_s}s ($attempt attempts, each kept in attempts/)." >&2
  return 124
}

# await_paint FRAME REFERENCE WHAT [CORRAL-FLAG...] -- FRAMEDIFF-FLAG...
#
# Screenshots until `framediff.py REFERENCE <shot> FRAMEDIFF-FLAG...` holds,
# then keeps that shot as $out/FRAME. On timeout the last shot taken is kept
# as FRAME anyway, so the artifact and any gate after this show the screen the
# wait gave up on rather than nothing. Returns 0, or 124 on timeout.
await_paint() {
  local frame="$1" reference="$2" what="$3"
  shift 3
  local shot_flags=()
  while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do
    shot_flags+=("$1")
    shift
  done
  shift
  local deadline=$(( SECONDS + settle_budget_s )) attempt=0 status candidate log last_shot=""
  mkdir -p "$attempts_dir"
  while :; do
    attempt=$(( attempt + 1 ))
    candidate="$attempts_dir/${frame%.png}-$(printf '%02d' "$attempt").png"
    log="${candidate%.png}.txt"
    status=0
    host_corral_within "$(attempt_budget "$deadline" "$corral_budget_s")" \
      screenshot "$vm" -o "$candidate" "${shot_flags[@]}" >"$log" 2>&1 || status=$?
    if [ "$status" -eq 0 ]; then
      last_shot="$candidate"
      python3 scripts/vmtest/framediff.py "$reference" "$candidate" "$@" >>"$log" 2>&1 \
        || status=$?
    fi
    if [ "$status" -eq 0 ]; then
      cp "$candidate" "$out/$frame"
      echo "ok: $what (attempt $attempt, kept as $frame)"
      tail -n 2 "$log"
      return 0
    fi
    echo "  attempt $attempt: not yet: $(tail -n 1 "$log")"
    if [ "$SECONDS" -ge "$deadline" ]; then
      break
    fi
    sleep "$poll_s"
  done
  if [ -n "$last_shot" ]; then
    cp "$last_shot" "$out/$frame"
  fi
  echo "--- the last attempt ---"
  cat "$log"
  echo "FAIL: TIMEOUT: $what was not on screen within ${settle_budget_s}s ($attempt attempts, each kept in attempts/; the last frame taken is $frame)." >&2
  return 124
}

# await_guest SLUG WHAT BUDGET SUBCOMMAND [ARG...]
#
# Polls the read-only `checks.sh SUBCOMMAND ARG...` until it exits 0, within
# BUDGET seconds. Returns 0 when it does; 255 at once when the guest stops
# answering SSH; 124 when the budget runs out. For a state of the session
# rather than of the launcher, so there is no launcher to be alive or dead.
await_guest() {
  local slug="$1" what="$2" budget="$3"
  shift 3
  local deadline=$(( SECONDS + budget )) attempt=0 status log
  mkdir -p "$attempts_dir"
  while :; do
    attempt=$(( attempt + 1 ))
    log="$attempts_dir/$slug-$(printf '%02d' "$attempt").txt"
    status=0
    guest_within "$(attempt_budget "$deadline" "$probe_budget_s")" \
      "$checks" "$@" >"$log" 2>&1 || status=$?
    if [ "$status" -eq 0 ]; then
      echo "ok: $what (attempt $attempt)"
      cat "$log"
      return 0
    fi
    if [ "$status" -eq 255 ]; then
      cat "$log"
      echo "FAIL: the guest stopped answering SSH while waiting for $what." >&2
      return 255
    fi
    echo "  attempt $attempt: not yet: $(tail -n 1 "$log")"
    if [ "$SECONDS" -ge "$deadline" ]; then
      break
    fi
    sleep "$poll_s"
  done
  echo "--- the last attempt ---"
  cat "$log"
  echo "FAIL: TIMEOUT: $what did not happen within ${budget}s ($attempt attempts, each kept in attempts/)." >&2
  return 124
}

# await_still FRAME WHAT -- FRAMEDIFF-FLAG...
#
# Screenshots until $stable_shots frames in a row are alike -- each one passes
# `framediff.py PREVIOUS THIS FRAMEDIFF-FLAG...` against the one before it --
# then keeps the last as $out/FRAME. Returns 0, or 124 on timeout, keeping the
# last shot as FRAME so the artifact shows what the wait gave up on.
#
# THIS IS THE ONE WAIT WHERE "STOPPED CHANGING" IS THE CONDITION, and only
# because there is nothing to compare against: these are the frames every
# later gate is judged from. Stillness alone would also accept a session that
# has not started drawing (run 42074769175's black desktop held still), which
# is why every caller first waits for the shell to say it has started
# (await_guest ... shell-started), a state rather than a picture.
await_still() {
  local frame="$1" what="$2"
  shift 2
  if [ "${1:-}" = "--" ]; then shift; fi
  local deadline=$(( SECONDS + settle_budget_s )) attempt=0 alike=0 status
  local candidate log previous="" last_shot=""
  mkdir -p "$attempts_dir"
  while :; do
    attempt=$(( attempt + 1 ))
    candidate="$attempts_dir/${frame%.png}-$(printf '%02d' "$attempt").png"
    log="${candidate%.png}.txt"
    status=0
    host_corral_within "$(attempt_budget "$deadline" "$corral_budget_s")" \
      screenshot "$vm" -o "$candidate" >"$log" 2>&1 || status=$?
    if [ "$status" -ne 0 ]; then
      # No frame, so nothing to be alike: the run of alike frames starts over.
      alike=0
      previous=""
    elif [ -z "$previous" ]; then
      alike=1
    elif python3 scripts/vmtest/framediff.py "$previous" "$candidate" "$@" >>"$log" 2>&1; then
      alike=$(( alike + 1 ))
    else
      alike=1
    fi
    if [ "$status" -eq 0 ]; then
      previous="$candidate"
      last_shot="$candidate"
    fi
    if [ "$alike" -ge "$stable_shots" ]; then
      cp "$candidate" "$out/$frame"
      echo "ok: $what held still (attempt $attempt, $alike frames alike, kept as $frame)"
      tail -n 2 "$log"
      return 0
    fi
    echo "  attempt $attempt: not yet still ($alike of $stable_shots alike): $(tail -n 1 "$log")"
    if [ "$SECONDS" -ge "$deadline" ]; then
      break
    fi
    sleep "$poll_s"
  done
  if [ -n "$last_shot" ]; then
    cp "$last_shot" "$out/$frame"
  fi
  echo "--- the last attempt ---"
  cat "$log"
  echo "FAIL: TIMEOUT: $what did not hold still within ${settle_budget_s}s ($attempt attempts, each kept in attempts/; the last frame taken is $frame)." >&2
  return 124
}

# THE CONTAINMENT BOX IS DERIVED FROM THE WINDOW, NOT WRITTEN DOWN TWICE.
#
# `--expect-box` asserts the changed region lies inside the window, so the box
# is a property of the window's size. It used to be four literals -- 300 140
# 980 800 -- taken from a 640x480 centred window. When the card was rewritten
# to 720x560 the window grew past its own gate's box and the run failed with
#
#   FAIL: changed region x 280..999 y 159..369 is not inside 300,140..980,800
#
# which is the gate working: 280..999 is exactly a 720-wide window centred on a
# 1280-wide screen, to the pixel. Widening the literals until it passed would
# have been the loosening this file warns against twice already, and would have
# left the same trap for the next person to change the geometry.
#
# So the box is computed from `design::GEOMETRY`, read out of the source. A
# future resize moves the gate with it instead of breaking it, and a rename
# that stops this finding the constants fails loudly below rather than silently
# asserting nothing.
#
# MARGIN is slack on each side for the drop shadow and for subpixel placement.
# It is not room to be wrong about where the window is: at 720 wide on a 1280
# screen the box is still 740 of 1280, so a launcher painting at either edge of
# the screen fails containment exactly as before.
screen_w=1280
screen_h=800
margin=20
design=crates/compass-ui/src/design.rs
# `exit` inside a command substitution leaves only the subshell, so the check
# is here rather than in the function -- a rename would otherwise reach the
# arithmetic below as an empty string.
geometry_field() {
  sed -n "s/^[[:space:]]*$1: \([0-9]\+\),.*/\1/p" "$design" | head -1
}
card_w=$(geometry_field card_width)
card_h=$(geometry_field card_max_height)
shadow_pad=$(sed -n 's/^pub const SHADOW_PADDING: u16 = \([0-9]\+\);.*/\1/p' "$design" | head -1)
# The window is the card plus its drop-shadow padding on all sides (design.rs
# SHADOW_PADDING=24, SHADOW_OFFSET_Y=6, SHADOW_BLUR=18). The card's 720x560 plus 24px padding
# becomes a 768x608 transparent window, centred, with the shadow inside the
# padding. The containment box must track the WINDOW, not just the card, or
# the 48px of padding fails the gate exactly as run 35568015733 did:
#   changed region x 256..1023 (768w centred) not inside 260..1020 (720w centred)
# No fallback to 0: run 36026959306 read nothing here, gated on the bare card,
# and failed on the very 768px window the comment above describes.
for field in card_w card_h shadow_pad; do
  if [ -z "${!field}" ]; then
    echo "launcher.sh: could not read the window geometry from $design" >&2
    exit 1
  fi
done
win_w=$(( card_w + 2 * shadow_pad ))
win_h=$(( card_h + 2 * shadow_pad ))
box_x0=$(( (screen_w - win_w) / 2 - margin ))
box_x1=$(( box_x0 + win_w + 2 * margin ))
# `if`, not `[ ... ] && ...`: under `set -e` a false test makes the whole list
# return 1 and takes the script with it.
if [ "$box_x0" -lt 0 ]; then box_x0=0; fi
if [ "$box_x1" -gt "$screen_w" ]; then box_x1=$screen_w; fi

# ONLY THE HORIZONTAL BOUND COMES FROM THE WINDOW. The vertical one is the
# whole screen below the top bar, and that is not slack -- it is what the two
# gates using this box actually compare.
#
# The first draft derived both axes from the window and produced y 140..700 for
# a 720x560 card. The open gate failed with
#
#   FAIL: changed region x 280..999 y 140..796 is not inside 260,140..1020,700
#
# and the region was right: the open gate and the came-back gate do NOT ignore
# the bottom strip the way steps 3a and 3d4b do, so the dock reacting to a new
# window is a legitimate part of what changed. The literals this replaced ended
# at 800 for exactly that reason, and tightening to the window silently changed
# what the gate meant.
#
# So: horizontally the box tracks the card, which is the assertion worth having
# -- a launcher painting at either edge of the screen still fails. Vertically it
# is the region the comparison looks at, which is a property of the ignore
# strips rather than of the card.
top_bar_h=140
box_y0=$top_bar_h
box_y1=$screen_h
# A containment box that covers the screen asserts nothing. That is not
# hypothetical: at a card width of 1240 the derivation above clamps to
# 0..1280 and the gate silently stops being a gate. Refuse rather than pass.
box_w=$(( box_x1 - box_x0 ))
if [ $(( box_w * 100 / screen_w )) -gt 80 ]; then
  echo "launcher.sh: a ${win_w}px window leaves a containment box ${box_w} of" \
       "${screen_w} wide, which asserts almost nothing. Widen the screen or" \
       "narrow the card before trusting this gate." >&2
  exit 1
fi
expect_box=(--expect-box "$box_x0" "$box_y0" "$box_x1" "$box_y1")
echo "window ${win_w}x${win_h} centred on ${screen_w}x${screen_h}" \
     "-> containment box ${box_x0},${box_y0}..${box_x1},${box_y1}"

echo "=== 0-. wait for GNOME Shell to finish starting (#345) ==="
# THE BOOT MARKER IS NOT "THE DESKTOP HAS PAINTED", AND ONE RUN SHOWED IT.
#
# corral is ready when wait-graphical.sh says logind has a graphical session,
# plus ten seconds. GNOME Shell is not finished by then on every run. In run
# 42074769175 it logged "GNOME Shell started" at 18:05:56.7, four seconds after
# corral returned, and everything below had already happened:
#
#   0    the login frame was a black desktop under the top bar (deviation
#        0.0340, against 0.1203 on a good run), the wallpaper not loaded yet;
#   0a   OverviewActive read false, because the shell's startup animation had
#        not opened the overview yet, so dismissing it did nothing;
#   0a2  the "bare desktop" was the same black frame;
#   0c   by now the startup animation HAD opened the overview, and it stayed;
#   0d   100% of the judged pixels differed, black against the overview.
#
# The shell logs that line from its startup-complete handler, which runs after
# the backgrounds have loaded and the startup animation has finished. So it is
# waited for as a state, read from the journal (checks.sh shell-started), and
# only then is the screen looked at.
await_guest shell-started "the session's GNOME Shell to finish starting up" \
  "$session_budget_s" shell-started

echo
echo "=== 0. what the session is actually showing at login ==="
# Evidence only, and kept separate from the bare desktop below because they
# turned out not to be the same picture. Taken once the frame holds still, so
# it shows the session's resting state rather than its startup animation.
await_still launcher-00z-login.png "the login frame" \
  -- --max-percent 0.1 --ignore-box 0 0 1279 139 --ignore-box 0 700 1279 799

echo
echo "=== 0a. put the session on a bare desktop (the Activities overview) ==="
# GNOME Shell opens the overview at login when the session has no windows.
# Until the first-run tour was suppressed the session always had one, so this
# never came up; the first run without the tour shows what it costs. Three
# things in that run's own frames, none of which the gates could name:
#
#   launcher-01-open.png    our window rendered as a SCALED THUMBNAIL inside a
#                           workspace tile, because it opened behind the
#                           overview. The gate at 3d passed on it -- at 34% in
#                           the expected box -- while measuring a preview.
#   launcher-04-hidden.png  the query from step 3 typed into GNOME's OWN search
#                           field, with GNOME's own results under it.
#   the 3d4 assertion       81.60% against a 3% bound, which is what finally
#                           failed, and the only reason any of this was seen.
#
# Dismissed in the guest over the session bus (checks.sh overview-dismiss), not
# by injecting Escape from the host, so the step can read back whether it
# worked instead of hoping.
guest "$checks" overview-dismiss

echo
echo "=== 0a2. the bare desktop, before anything of ours runs ==="
# The reference step 0d judges the engine against, so it is taken once the
# overview's closing animation has finished: when $stable_shots frames in a row
# agree, within a tenth of 0d's 1% bound, over the region 0d judges (the same
# two strips of shell furniture are left out, for the reasons given at 0d).
await_still launcher-00a-bare-desktop.png "the bare desktop" \
  -- --max-percent 0.1 --ignore-box 0 0 1279 139 --ignore-box 0 700 1279 799

echo
echo "=== 0b. start the engine, so the launcher has something to attach to ==="
# ADR-0015 made the window resident and driven: `compass ui` connects to
# `compass serve` and waits to be told to show. Order matters -- a launcher
# started first comes up undriven, and every summon below is then refused
# correctly and confusingly.
guest "$checks" engine-start

echo
echo "=== 0b2. did the engine find any applications? (the gate, #95) ==="
# Placed here rather than with the pixel gates because it is about the engine, not the screen,
# and because failing here says "the index is empty" instead of letting it present three steps
# later as a launcher that draws fine and finds nothing.
guest "$checks" engine-index

echo
echo "=== 0b3. are the machine's Flatpak applications in the index? (the gate, #105) ==="
# The index being non-empty says nothing about Flatpaks: on this image every
# host application comes from the base OS, and until now the system and user
# Flatpak roots were both empty, so the code path #105 lived in was never run
# here.
#
# A Flatpak export is a symlink into the deploy tree, and our sandbox granted
# the exports directory without it -- which made every Flatpak on a user's
# machine invisible, silently. The image now installs one application into each
# root and this asks the running engine for them by name.
#
# Gated. There is no threshold to calibrate and nothing to record first: either
# the query returns the application or the sandbox cannot see it.
guest "$checks" flatpak-apps

echo
echo "=== 0c. the desktop with the engine up (THE CONTROL for every gate) ==="
# THE CONTROL MOVED, AND THAT IS THE POINT.
#
# It used to be taken before anything of ours ran, when the only thing that
# happened between it and the launcher frame was the launcher. Starting the
# engine in between broke that: the gate at 3d asks "did a LAUNCHER window
# appear", and it was being shown every pixel the engine changed as well.
#
# It cost a run. The gate failed with a changed region 962 px wide against a
# 640 px window -- while the launcher's own log showed it configuring a surface
# at exactly 640x480, so the window was never the problem. `serve` binds the
# GlobalShortcuts portal at startup, and the portal asks the user for
# permission; whatever that puts on screen was landing inside a comparison that
# claimed to be about the launcher.
#
# Taking the control AFTER the engine is up restores what the gate means: the
# only thing that differs between this frame and the next is the launcher. The
# thresholds and boxes below are untouched -- this is not a gate loosened to
# fit a failure, it is a control put back where it belongs.
shot "launcher-00-before.png"

echo
echo "=== 0d. starting the engine must not put anything on screen (the gate) ==="
# THIS WAS A RECORDING AND IS NOW A GATE, on the strength of what it recorded.
#
# It was added ungated to answer a question two failed runs could only infer:
# what does starting the engine draw? The answer came back 1.62% of pixels in a
# 496x532 box -- GNOME asking the user to grant the launcher hotkey, which
# `serve` requests from the GlobalShortcuts portal at startup.
#
# So `engine-start` now passes `--no-hotkey`, and with nothing to ask about,
# starting the engine should be invisible. That makes this a real assertion
# about the flag: if it stops working, the dialog comes back and this fails
# here, naming the cause, instead of corrupting the launcher gate three steps
# later with a bounding box nobody can interpret.
#
# TWO STRIPS OF SHELL FURNITURE ARE EXCLUDED, top and bottom, and neither is
# ours to keep still.
#
# The top bar carries a clock, and a minute boundary is not the engine drawing
# something -- that one cost a red run back when the launcher gate was written.
#
# The bottom is the dash, and it cost this gate its first run: 2.04% in a
# 237x84 box along the bottom edge. Cropping both frames to that region and
# looking at them shows the dash's ICONS are identical -- Files, Trash, the app
# grid, unchanged -- and what moves is the panel backdrop behind them, which
# the shell redraws when a process starts. Nothing we can prevent and nothing
# worth failing over.
#
# What is left between the strips is the part that matters: if the hotkey
# dialog ever comes back it lands in the middle of the screen, where this gate
# is still sharp.
#
# Binding the hotkey for real is Spike A's job, in its own VM, where a dialog
# is the expected outcome rather than contamination.
python3 scripts/vmtest/framediff.py \
  "$out/launcher-00a-bare-desktop.png" "$out/launcher-00-before.png" \
  --max-percent 1 --ignore-box 0 0 1279 139 --ignore-box 0 700 1279 799

echo
echo "=== 1. open the launcher in the session ==="
guest "$checks" launcher-start

echo
echo "=== 1b. what is the launcher blocked on? ==="
# Added after the instrumented run: the log stops 200ms in and the process sits
# there alive, so logging cannot say where it stopped — the stuck code is not
# the code doing the logging. The kernel can. Best-effort; a diagnostic must
# never fail the run it exists to explain.
guest "$checks" launcher-diagnose || true

echo
echo "=== 2. the screen with the launcher open ==="
shot "launcher-01-open.png" --require-paint

echo
echo "=== 3. type a query at it ==="
# THIS STEP NOW ASSERTS SOMETHING, and the history of why it did not is worth
# keeping, because the comment has been wrong twice and each correction was paid
# for by a run.
#
# It first said injection "does not reach this session at all"; the evdev
# capture disproved that, and the chain is longer:
#
#   QEMU delivers the scancode          yes — 288 bytes captured on the evdev
#                                        node while the key was injected
#   the guest kernel sees it            yes — same evidence
#   gnome-shell holds the keyboard      yes — event0..event3 open on its fds
#   the portal grants the binding       yes — "portal available | bind granted"
#   the shortcut fires                  NO  — "activated False"
#
# So the loss is above the kernel, in a session whose compositor is holding the
# device it is losing events from. "Does not reach the session" would point at
# corral or QEMU, which the capture rules out.
#
# The seat is ruled out too, as of the run that first printed the untruncated
# diagnostic: the AT keyboard IS on seat0, and the capture decodes clean
# LEFTMETA press and release on event1 -- the very node gnome-shell holds open.
#
# THE PARAGRAPH THAT USED TO BE HERE WAS WRONG, and the run that suppressed the
# first-run tour disproved it with a screenshot.
#
# It argued from "pressing Super ALONE leaves the framebuffer byte-identical"
# that "the compositor is inert to injected input generally", and went on to
# suspect logind of having the session's devices paused -- which would have been
# a deep problem and was not one. In that run's `launcher-04-hidden.png` the
# query from THIS STEP is sitting in GNOME Shell's own search field with GNOME's
# own results rendered under it. The compositor receives injected keys and acts
# on them.
#
# What was actually inert was the session, because the tour dialog held the
# keyboard: Super does nothing while a modal dialog has the grab, and the
# framebuffer stays byte-identical for that reason rather than for the one the
# paragraph gave. Every layer in the table above was measured correctly; only
# the last line's cause was wrong.
#
# So "activated False" for our own shortcut is still unexplained and is still
# Spike A's question -- but it is no longer explained by input not arriving, and
# the next person should not spend a run on logind.
#
# With the overview dismissed and the tour gone, the keys reach whatever holds
# the focus, and our window is the only window on the desktop. On the run that
# first got there, `launcher-02-typed.png` came back BYTE-IDENTICAL to
# `launcher-01-open.png`: the launcher drew a search box and ignored the
# keyboard. That was #91, and the cause was in our code -- Iced delivers typed
# characters only to a `text_input` that holds widget focus, and nothing ever
# focused ours. A person trying the launcher clicks the box without noticing
# they did; the tier only types, which is why the tier found it.
#
# So the gate below is the regression check for that fix, and the floor is
# deliberately near zero rather than fitted. What is being asserted is "the
# keystroke reached our field at all", which is threshold-free; how many pixels
# two characters and a results list move has not been measured yet, and ADR-0010
# is explicit that inventing a number before seeing one is how a tier starts
# flaking. The run prints the real figure, and the floor can be raised from data
# in a two-line change once a few runs agree.
#
# The box and the ignored strips are the launcher gate's, for the same reasons
# given there: the region that may change is the window, and the clock and the
# dash are not ours to keep still.
#
# CONTROLLED, on the published artifact of run 35285035604 rather than on a
# claim: these exact arguments against that run's two frames exit 1 with
# "FAIL: only 0.00% of pixels changed, expected at least 0.10%".
host_corral type "$vm" "$query"
shot "launcher-02-typed.png"

echo
echo "=== 3a. did the query reach OUR field? (the gate, #91) ==="
python3 scripts/vmtest/framediff.py \
  "$out/launcher-01-open.png" "$out/launcher-02-typed.png" \
  --min-percent 0.1 "${expect_box[@]}" \
  --ignore-box 0 0 1279 139 --ignore-box 0 700 1279 799

echo
echo "=== 3a2. and what does the launcher say it did? (the exact version of 3a) ==="
# 3a proves a keystroke changed the screen inside our window. It cannot say the
# query reached the FIELD rather than, say, a tooltip -- that was read out of
# the frames by eye once and has been assumed since. This asserts it.
#
# LAUNCHER_QUERY is what the step above typed, so the assertion follows the
# fixture instead of restating it.
#
# THE BACKSLASHES ARE LOAD-BEARING. `guest` is ssh, and ssh joins its argv into
# one string that the REMOTE shell parses again -- so a plain "query=\"fi\""
# arrives as `query=fi`, the quotes eaten in transit, and the gate looks for a
# string the log does not contain. Verified against a shell that re-parses its
# argv the way sshd does, rather than discovered twenty-five minutes into a VM
# run. `\\"` survives both passes and arrives as `"`.
#
# The quotes cannot simply be dropped: `panel_title=Open` without them is a
# prefix of `panel_title=Open in New Window`, which is exactly the distinction
# run 199 had to read out of a screenshot by eye.
guest "$checks" ui-state "query=\\\"$query\\\"" "window=open"

echo
echo "=== 3b. and is it blocked in the same place after the keystroke? ==="
# The same probe again, deliberately. A process parked in the same syscall on
# the same socket both times is stuck; one that has moved is merely slow, and
# those two want completely different fixes.
guest "$checks" launcher-diagnose || true

echo
echo "=== 3b2. what does the launcher cost at idle? (Phase 1 gate) ==="
# Phase 1's gate says "idle RSS < 30 MB" and nobody had measured it, because
# until the launcher drew there was nothing to measure. Taken here, after the
# window is up and before the control application starts competing for memory.
guest "$checks" launcher-rss

echo "=== 3b3. what does cold start cost? (#13 §8.5, recorded) ==="
# The first number for a row of §8.5 that has never had one. Gated only on the
# line existing: a launcher that drew nothing, or instrumentation that was
# removed, are both real findings. The figure itself is recorded.
guest "$checks" cold-start

echo "=== 3d. did a launcher window actually appear? (the gate) ==="
# The first assertion in this tier derived from a measurement rather than from
# an assumption, and the reason it exists is that its absence let a wrong
# conclusion stand for three runs.
#
# corral's luminance deviation cannot answer "did a window appear": five frames
# across two jobs all read 0.1564 to four decimal places while showing visibly
# different things, one of them containing a 640x480 launcher. Pixels can, and
# framediff.py counts them.
#
# The numbers are taken from two consecutive runs that agreed to the pixel:
# 84150 changed (8.22%) in a box at x 335..942, y 152..796, against a window
# configured 640x480 centred, i.e. x 320..960, y 160..640. The gate is set well
# below and around that — 3% rather than 8.22% — because the point is to catch
# "nothing was drawn", not to pin the exact pixels of a theme. A tighter bound
# would break on the first font change and teach everyone to ignore it.
#
# The box is no longer written here; it is derived from `design::GEOMETRY` at
# the top of this file, and at 640x480 that derivation reproduces the literals
# this paragraph was written against. The percentage floor stays a measurement,
# because it is one; the box is geometry, and geometry can be computed.
#
# THE TOP BAR IS IGNORED, and that is not a loosening of the gate.
#
# --expect-box asserts the changed region lies WITHIN the box, which silently
# also asserts that nothing else on the screen changed between the two frames.
# That was never the intent, and it is not true: GNOME's top bar carries a
# clock. It cost a red run to find out, and the two runs say exactly what
# happened:
#
#   passing   84077 changed  box x 335..942  y 152..796
#   failing   84172 changed  box x 335..942  y  10..796
#
# Ninety-five extra pixels, in the top bar, with the launcher's own footprint
# identical to the pixel on both axes. A clock digit turning over dragged miny
# from 152 to 10 and failed a gate about whether a window appeared. Two
# calibration runs happened not to cross a minute boundary; this one did.
#
# So the shell's own furniture is excluded from the comparison rather than the
# box being widened to swallow it. Widening would have made "the launcher
# painted at the top of the screen" pass, which is a real failure this gate
# should keep catching. Excluding y < 140 keeps the containment assertion sharp
# for the whole region the launcher can legitimately occupy, and --min-percent
# still requires a real window's worth of pixels inside it: a launcher that
# painted only in the ignored strip would now change ~0% and fail there.
python3 scripts/vmtest/framediff.py \
  "$out/launcher-00-before.png" "$out/launcher-01-open.png" \
  --min-percent 3 "${expect_box[@]}" --ignore-box 0 0 1279 139

echo
echo "=== 3d2. can the engine hide and summon the window? (the ADR-0015 gate) ==="
# THE THING THIS TIER COULD NOT ASSERT BEFORE.
#
# Every check above can see a *process*. None of them can see a *connection*.
# `serve` refuses show/hide/toggle when no window has attached, so a `toggle`
# that succeeds is proof of the whole chain at once: CLI, socket, engine, the
# window link, and a window that answered on the other end of it.
#
# The keypress leg is still missing and is still not this job's to fix:
# injected input does not reach this compositor (see step 3's note), so the
# client here is `compass` rather than Super+Space. What that leaves untested
# is the portal delivering an activation. Everything after the activation is
# exercised.
guest "$checks" window-attached
# THE SAME SETTLE THE SUMMON PATH GETS, AND FOR THE MIRROR-IMAGE REASON.
#
# This screenshot used to be taken the instant the toggle was acknowledged. The
# toggle returning means the window was *told* to hide, not that the compositor
# has finished un-mapping it and repainted the desktop underneath -- and under
# llvmpipe that is not instant, which is exactly what the comment on the summon
# side below says about painting.
#
# So the gate below was racing, and it lost on a17680e: `launcher-04-hidden.png`
# and `launcher-05-summoned.png` both came back with corral deviation 0.3842,
# identical to four decimals, because the "hidden" frame still had the launcher
# in it. The assertion then reported 55.85% of pixels changed against a 3%
# ceiling and read as "hide is broken", which it was not.
#
# Symmetry is the fix: the appearing frame is waited for, so the disappearing
# one must be too.
#
# Waited for, not slept through (#333; see "WAITING FOR A TRANSITION" at the
# top). The paint condition is the went-away gate's own, against the same
# control, and that is deliberate rather than circular. "Changed since the
# launcher was last on screen" is already true of a frame taken part-way
# through GNOME's close animation, with the window still partly drawn -- which
# would put this gate back in the race it lost on a17680e. The end state is
# what is waited for, and the end state is "looks like the desktop did". The
# paired gates in 3d4 still judge the kept frame, and the summon half is what
# stops a screen that never showed a launcher from passing both.
await_state hide "the launcher to report its window hidden" "window=hidden"
await_paint launcher-04-hidden.png "$out/launcher-00-before.png" \
  "the desktop, repainted without the launcher" -- \
  --max-percent 3 --ignore-box 0 0 1279 139 --ignore-box 0 700 1279 799

echo
echo "=== 3d3. and does summoning it bring the window back? ==="
guest "$checks" summon
# The same settle the open path gets. Opening a surface under llvmpipe is not
# instant, and screenshotting before the paint would produce "summon does not
# work" for the same reason step 1 produced "the launcher does not draw" --
# twice, wrongly.
#
# Waited for rather than slept through. The paint condition is the open gate's
# floor and box, measured against the HIDDEN frame rather than the control: it
# asks whether the window came back since it went, which the came-back gate in
# 3d4 -- measured against the control -- does not.
await_state summon "the launcher to report its window open" "window=open"
await_paint launcher-05-summoned.png "$out/launcher-04-hidden.png" \
  "the launcher, painted again over the desktop" --require-paint -- \
  --min-percent 3 "${expect_box[@]}" --ignore-box 0 0 1279 139

echo
echo "=== 3d4. the two assertions that make the pair mean something ==="
# Hidden must look like the bare desktop, and summoned must look like the
# launcher again. Either alone is weak: a frame that never changes passes the
# first, and a frame that never changes fails the second, so the pair together
# is what says the window actually went away and actually came back.
#
# Same box and the same ignore strip as the gate above, for the same reasons --
# including the top-bar clock, which cost a red run to discover.
echo "--- the window went away: hidden should match the desktop before it opened ---"
# Same two strips as step 0d, for the same reason: the dash gains an entry
# while the launcher process is resident, and stays changed after its window is
# gone. That is correct -- the process really is still running, which is the
# whole point of ADR-0015 -- so it must not be read as "the window is still
# there".
python3 scripts/vmtest/framediff.py \
  "$out/launcher-00-before.png" "$out/launcher-04-hidden.png" \
  --max-percent 3 --ignore-box 0 0 1279 139 --ignore-box 0 700 1279 799

echo "--- and came back: summoned should look like the launcher did ---"
python3 scripts/vmtest/framediff.py \
  "$out/launcher-00-before.png" "$out/launcher-05-summoned.png" \
  --min-percent 3 "${expect_box[@]}" --ignore-box 0 0 1279 139

echo
echo "=== 3d4b. does Ctrl+B open the action panel? (recorded again, see below) ==="
# THE VIEW WORK'S FIRST EXPOSURE TO A REAL SESSION, and deliberately not a gate
# on its first outing.
#
# Everything above answers "does the launcher paint, and do keystrokes reach
# it". Nothing above answers "is what it paints the right thing" -- and the
# action panel is the one piece of view work with a keystroke of its own, so it
# is the one piece this tier can reach at all. Until this step existed, the
# panel had no verification outside its own unit tests, and the PR description
# said otherwise for a while.
#
# Ctrl+B and not Ctrl+K: the C++ binds Ctrl+K on macOS only, because Ctrl+K is
# the vim "move up" chord everywhere else. `keybind-manager.cpp` is the source
# for that and the Rust engine follows it.
#
# `corral key` presses one combination together (QMP send-key with both qcodes),
# which is what a chord needs -- `corral type` would send the characters one
# after another and never hold the modifier.
#
# IT RAN UNGATED FIRST, and the two runs it took to earn the threshold are the
# reason there is one. ADR-0010 forbids inventing a floor before seeing a
# number, and #91 is the standing proof that a wrong assertion here costs runs
# rather than finding bugs.
#
# Runs 199 (3da5283) and 200 (317940f) both printed, from separate VM boots:
#
#   2697 of 716800 pixels differ (0.38%)  box x 360..466 (107w) y 315..447 (133h)
#
# Byte-identical, not merely close, which is what makes a floor defensible after
# two runs rather than a dozen: there is no spread to fit to.
#
# The frames were read rather than trusted. In that box the root list
# (`> Firewall / Files / New Window`) is replaced by an `Actions` header,
# `> Open`, a `---` divider, a `Copy` section header, and `Copy name` /
# `Copy path` -- the flattened structure `compass_ui::action_panel` is tested
# against, with the caret on the first SELECTABLE row rather than the header.
#
# The floor was 0.1%, about a quarter of what was measured, deliberately NOT
# fitted to 0.38%.
#
# **UNGATED AGAIN, ON PURPOSE, AND THIS IS NOT A RETREAT.** The launcher's view
# was rewritten to draw an Adwaita card -- icons, subtitles, a filled selection
# rather than a caret, and the panel *floating over* the list instead of
# replacing it. Every one of those changes the number and the box this
# assertion was built from: a panel that no longer replaces the list is a
# different region of a different size. Holding the old threshold over new
# geometry would be asserting a measurement nobody has taken, which is the
# thing ADR-0010 forbids and the thing the recorded-first discipline exists to
# avoid. So it records again, the next run produces the new figures, and the
# gate is re-earned from those exactly as it was earned the first time.
#
# What must NOT happen is the floor being nudged until it passes. If the new
# figure is near 0.00 the chord stopped arriving and that is a bug in the
# rewrite, not a threshold to tune.
if host_corral key "$vm" ctrl b; then
  # The same wait the hide and summon paths get, for the same reason: a
  # screenshot taken before the paint reads as "the panel does not open". The
  # paint condition is the recorded floor below, against the summoned frame.
  #
  # Neither half is a gate here. This step records, and 3d4c gates on the
  # launcher's own account; a wait that runs out says so, and the figure from
  # the last frame taken is printed anyway, because a figure near zero is the
  # finding.
  await_state panel "the launcher to report the action panel open" "panel=open" \
    || echo "NOT GATED HERE: the launcher did not report the panel open, and 3d4c below gates on exactly that."
  await_paint launcher-06-panel.png "$out/launcher-05-summoned.png" \
    "the action panel, painted over the list" -- \
    --min-percent 0.1 "${expect_box[@]}" \
    --ignore-box 0 0 1279 139 --ignore-box 0 700 1279 799 \
    || echo "NOT GATED: the panel's paint did not arrive within the wait; the figure below is from the last frame taken."
  if [ -f "$out/launcher-06-panel.png" ]; then
    python3 scripts/vmtest/framediff.py \
      "$out/launcher-05-summoned.png" "$out/launcher-06-panel.png" \
      --min-percent 0.1 "${expect_box[@]}" \
      --ignore-box 0 0 1279 139 --ignore-box 0 700 1279 799 \
      || echo "NOT GATED: read the figure above against the new card geometry before re-gating."
  else
    echo "NOT GATED: could not screenshot after the chord."
  fi
else
  echo "NOT GATED: corral could not send the chord. That says nothing about the panel."
fi

echo
echo "=== 3d4c. did the panel open, in the launcher's own words? (the gate) ==="
# The framediff above is deliberately ungated while the card geometry settles.
# This is not: whether Ctrl+B reached the application and opened the panel is a
# question about the state machine, not about pixels, and it has an exact
# answer that no threshold has to be fitted to.
#
# Outside the `if` above on purpose. Those branches are about whether corral
# could send a chord and take a picture; the launcher's own account of what it
# did survives either of them failing, and is worth having precisely then.
#
# `panel_title="Open"` is the first SELECTABLE row. Run 199's frames had to be
# read by eye to confirm the caret was not on the header above it; this is that
# same claim, checked rather than looked at.
guest "$checks" ui-state "panel=open" 'panel_title=\"Open\"'

echo
echo "=== 3d5. what did the engine make of the hotkey? (recorded, not gated) ==="
# Whether GNOME grants LOGO+space is the user's decision through a permission
# dialog, and an unattended session may well be refused. That is a real outcome
# worth reading in the log, not a failure of the code.
guest "$checks" hotkey-status || true

echo
echo "=== 3d6. can ANY client draw in this session? (the control) ==="
# MOVED AFTER THE SUMMON GATES, AND THE ORDER IS THE POINT. This starts a
# second application and leaves it on screen. Run before the gates above, it
# puts a whole file manager into frames that are compared against a control
# taken before it existed -- which is exactly how 3d4 failed at 20.38% in a
# 957px box, with a nautilus window sitting in the "hidden" frame.
#
# It is a diagnostic for 3d, not an input to it, so it loses nothing by
# running once the frame-comparing steps are done.
# The control this job should have had from the start. Everything above says
# our launcher puts no window on screen; none of it distinguishes that from
# nothing being able to. A session where no client can render would produce
# identical evidence and would exonerate the launcher entirely — and the
# desktop painting does not settle it, because that is GNOME Shell compositing
# its own furniture, not a client surface.
guest "$checks" control-app-start || true
shot "launcher-03-control-app.png"

echo
echo
echo "=== 3e. harvest a real desktop-entry corpus from this box ==="
# Phase 1's gate wants ~500 real entries for Suite 0 ranking parity and the
# corpus has 27, which §11.2 calls the binding constraint on the phase. The
# harvester asks for "a real desktop — ideally a Bluefin box"; this VM is one,
# booted fresh with a full GNOME application set on the target platform.
#
# It produces an artifact for a person to review and commit. It deliberately
# does not write into the repository: a corpus shapes every ranking assertion
# we make, and it should not grow by a job quietly appending to it.
guest "$checks" harvest-corpus || true
if guest_within "$probe_budget_s" test -f /tmp/corpus.tar.gz 2>/dev/null; then
  # `ssh cat > file` rather than scp: scp's protocol has been deprecated and
  # removed in places, and this needs no second tool.
  guest cat /tmp/corpus.tar.gz > "$out/corpus.tar.gz" 2>/dev/null \
    && echo "corpus.tar.gz retrieved ($(wc -c < "$out/corpus.tar.gz") bytes)" \
    || echo "(could not retrieve the corpus tarball)"
else
  echo "(no corpus tarball in the guest)"
fi

echo
echo "=== 4. is it still running? ==="
# A launcher that dies on the first keystroke is a real bug, and without this
# it would show up only as two screenshots that happen to look alike.
guest "$checks" launcher-status
