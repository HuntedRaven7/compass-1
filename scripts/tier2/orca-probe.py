#!/usr/bin/env python3
"""The Orca gate's driver (#305): a person using Orca, on real Mutter.

Run by `orca.sh`, under `headless-gnome-spike.sh`, with Orca already reading
the session and writing its debug log. The launcher has been started hidden;
this summons it with the command given after `--` and then does what a
screen-reader user does, with keys pressed through Mutter's own input path
(org.gnome.Mutter.RemoteDesktop, a virtual keyboard the compositor routes to
the focused surface exactly as a real one):

  1. summon       the window, the search field focused
  2. type         the query in the field, the results, their count announced
  3. arrow        the selection moves and the focus with it
  4. Ctrl+B       the action panel, its current action
  5. invoke       the panel's first action, pressed through AT-SPI
  6. form         a text field typed into, Tab between controls, a checkbox
                  toggled with Space
  7. hide         nothing of the window left on the bus

After each step it checks two things, and both must hold:

  * the AT-SPI tree, read the way Orca reads it (libatspi, the registry):
    roles, names, text, states;
  * what Orca SAID, from the `SPEECH OUTPUT:` lines of its debug log. No
    speech synthesiser runs here; Orca logs every utterance before handing
    it to speech-dispatcher, so the log is what a person would have heard.

A failed check prints what was on the bus and everything Orca said, and the
script exits 1. The transcript is also written to --transcript, as the
evidence the run leaves behind.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, Gio, GLib  # noqa: E402

# X keysyms. Printable ASCII is its own keysym.
KEY = {
    "Return": 0xFF0D,
    "Escape": 0xFF1B,
    "Tab": 0xFF09,
    "BackSpace": 0xFF08,
    "Up": 0xFF52,
    "Down": 0xFF54,
    "Control_L": 0xFFE3,
    "Shift_L": 0xFFE1,
}

WINDOW_NAME = "Compass"
SEARCH_NAME = "Search…"
STEP_TIMEOUT_S = 15.0
# Orca 50 puts a space before the voice's settings, Orca 51 does not.
SPEECH = re.compile(r" - SPEECH OUTPUT: '(.*)' ?(?:\{.*)?$")


class GateFailed(Exception):
    pass


class Keyboard:
    """A virtual keyboard from Mutter's RemoteDesktop service.

    The session lives as long as this process's bus connection: Mutter ends
    a session whose client vanishes, which is why this is not `gdbus call`.
    """

    SERVICE = "org.gnome.Mutter.RemoteDesktop"

    def __init__(self) -> None:
        self.bus = Gio.bus_get_sync(Gio.BusType.SESSION)
        (self.path,) = self._call(
            "/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop", "CreateSession"
        ).unpack()
        self._session("Start")

    def _call(self, path: str, iface: str, method: str, args=None):
        return self.bus.call_sync(
            self.SERVICE, path, iface, method, args, None, Gio.DBusCallFlags.NONE, 5000, None
        )

    def _session(self, method: str, args=None):
        return self._call(self.path, "org.gnome.Mutter.RemoteDesktop.Session", method, args)

    def _key(self, keysym: int, pressed: bool) -> None:
        self._session("NotifyKeyboardKeysym", GLib.Variant("(ub)", (keysym, pressed)))
        time.sleep(0.02)

    def press(self, *names: str) -> None:
        """A chord: every key down in order, then up in reverse."""
        syms = [KEY[n] if n in KEY else ord(n) for n in names]
        for sym in syms:
            self._key(sym, True)
        for sym in reversed(syms):
            self._key(sym, False)
        time.sleep(0.05)

    def type(self, text: str) -> None:
        """Types at a person's pace. Orca drops an announcement that comes
        within 0.1 s of the last one from the same application, so keys
        faster than that lose the result count of the final keystroke."""
        for char in text:
            if not (" " <= char <= "~") or char.isupper():
                raise ValueError(f"type() sends lowercase ASCII only, not {char!r}")
            self.press(char)
            time.sleep(0.15)


@dataclass
class Node:
    role: str
    name: str
    description: str
    states: set[str]
    text: str | None
    acc: Atspi.Accessible = field(repr=False)
    children: list["Node"] = field(default_factory=list, repr=False)

    def walk(self):
        yield self
        for child in self.children:
            yield from child.walk()

    def find(self, role: str, name: str | None = None) -> "Node | None":
        for node in self.walk():
            if node.role == role and (name is None or node.name == name):
                return node
        return None

    def all(self, role: str) -> list["Node"]:
        return [node for node in self.walk() if node.role == role]

    def focused(self) -> "Node | None":
        for node in self.walk():
            if "focused" in node.states:
                return node
        return None

    def render(self, depth: int = 0) -> str:
        text = f" text={self.text!r}" if self.text is not None else ""
        desc = f" desc={self.description!r}" if self.description else ""
        line = f"{'  ' * depth}{self.role} {self.name!r}{desc} [{' '.join(sorted(self.states))}]{text}"
        return "\n".join([line, *(child.render(depth + 1) for child in self.children)])


def read(acc: Atspi.Accessible) -> Node:
    states = {Atspi.StateType(s).value_nick for s in acc.get_state_set().get_states()}
    text = None
    iface = acc.get_text_iface()
    if iface is not None:
        text = Atspi.Text.get_text(iface, 0, Atspi.Text.get_character_count(iface))
    node = Node(
        role=acc.get_role_name(),
        name=acc.get_name() or "",
        description=acc.get_description() or "",
        states=states,
        text=text,
        acc=acc,
    )
    for index in range(acc.get_child_count()):
        child = acc.get_child_at_index(index)
        if child is not None:
            node.children.append(read(child))
    return node


def launcher_window() -> Node | None:
    """The launcher's frame as the registry has it now, or None."""
    while GLib.MainContext.default().iteration(False):
        pass
    desktop = Atspi.get_desktop(0)
    for index in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(index)
        if app is None:
            continue
        try:
            # Read fresh: libatspi caches what its events said, and this
            # process does not run the main loop that would keep it current.
            app.clear_cache()
            for child in range(app.get_child_count()):
                frame = app.get_child_at_index(child)
                if frame is not None and frame.get_name() == WINDOW_NAME:
                    return read(frame)
        except GLib.Error:
            continue
    return None


class Speech:
    """What Orca has said, from its debug log."""

    def __init__(self, path: Path) -> None:
        self.path = path
        self.mark = 0

    def all(self) -> list[str]:
        try:
            lines = self.path.read_text(errors="replace").splitlines()
        except FileNotFoundError:
            return []
        said = []
        for line in lines:
            match = SPEECH.search(line)
            if match:
                said.append(match.group(1))
        return said

    def since_mark(self) -> list[str]:
        return self.all()[self.mark :]

    def set_mark(self) -> None:
        self.mark = len(self.all())


@dataclass
class Gate:
    keyboard: Keyboard
    speech: Speech
    transcript: list[str] = field(default_factory=list)
    last_tree: str = "(nothing read yet)"

    def step(self, title: str) -> None:
        self.quiet()
        self.speech.set_mark()
        self.transcript.append(f"\n## {title}")
        print(f"\n=== {title} ===", flush=True)

    def quiet(self, still_s: float = 0.5, timeout: float = 5.0) -> None:
        """Waits until Orca has stopped writing, as a person waits for it to
        finish speaking. Orca works through events from a queue, and keys
        pressed while it is still on the last step's get their answers
        interleaved with that step's."""
        deadline = time.monotonic() + timeout
        size, since = -1, time.monotonic()
        while time.monotonic() < deadline:
            now = self.speech.path.stat().st_size if self.speech.path.exists() else 0
            if now != size:
                size, since = now, time.monotonic()
            elif time.monotonic() - since >= still_s:
                return
            time.sleep(0.1)

    def wait(self, what: str, check, timeout: float = STEP_TIMEOUT_S):
        """Polls `check(window, said)` until it returns something truthy."""
        deadline = time.monotonic() + timeout
        last_error = ""
        while time.monotonic() < deadline:
            window = launcher_window()
            self.last_tree = window.render() if window else "(no launcher window on the bus)"
            try:
                result = check(window, self.speech.since_mark())
            except (GLib.Error, AttributeError, IndexError) as error:
                result, last_error = None, str(error)
            if result:
                print(f"ok: {what}", flush=True)
                return result
            time.sleep(0.2)
        said = self.speech.since_mark()
        raise GateFailed(
            f"{what}: not within {timeout:.0f}s\n"
            f"--- the launcher on the bus ---\n{self.last_tree}\n"
            f"--- Orca said, this step ---\n" + "\n".join(f"  {s!r}" for s in said)
            + (f"\n--- last error ---\n{last_error}" if last_error else "")
        )

    def heard(self, what: str, predicate, timeout: float = STEP_TIMEOUT_S) -> str:
        """Waits for an utterance this step that satisfies `predicate`."""

        def check(_window, said):
            return next((s for s in said if predicate(s)), None)

        utterance = self.wait(f"Orca said {what}", check, timeout)
        self.transcript.append(f"Orca: {utterance!r}")
        return utterance

    def note_speech(self) -> None:
        for said in self.speech.since_mark():
            line = f"  said: {said!r}"
            if line not in self.transcript:
                self.transcript.append(line)


def results(window: Node | None) -> list[Node]:
    if window is None:
        return []
    box = window.find("list box", "Results")
    return box.all("list item") if box else []


def status(window: Node | None) -> str | None:
    if window is None:
        return None
    bar = window.find("status bar")
    return bar.name if bar else None


def run(args: argparse.Namespace, gate: Gate) -> None:
    gate.step("1. summon: the window opens with the search field focused")
    subprocess.run(args.summon, check=True, timeout=30)
    gate.wait("the launcher's window is on the bus", lambda w, _s: w)
    # The first key creates the virtual keyboard's capability on the seat,
    # and Mutter gives the window the keyboard then; a modifier on its own
    # types nothing.
    gate.keyboard.press("Shift_L")
    field_ = gate.wait(
        "the window is active and the search field has the focus",
        lambda w, _s: w
        and "active" in w.states
        and (f := w.focused())
        and f.role == "entry"
        and f.name == SEARCH_NAME
        and "editable" in f.states
        and f,
    )
    gate.transcript.append(f"bus: focused {field_.role} {field_.name!r}")
    # Orca 50 says the name and role as two utterances, Orca 51 as one.
    gate.heard("the search field's name", lambda s: SEARCH_NAME in s)
    gate.quiet()
    # The window opens on its field. A list Orca reads first is a selection
    # appearing under the field, which takes Orca out of it (see
    # compass_ui::accessibility, where the tree waits for the first answer).
    said = gate.speech.since_mark()
    first_field = next(i for i, s in enumerate(said) if SEARCH_NAME in s)
    if any(s.startswith("List with") for s in said[:first_field]):
        raise GateFailed(f"Orca read the results before the search field: {said}")
    gate.note_speech()

    gate.step(f"2. type: {args.query!r} in the field, the results and their count")
    gate.keyboard.type(args.query)

    def typed(w, _s):
        if w is None:
            return None
        focus = w.focused()
        items = results(w)
        count = status(w)
        return (
            focus is not None
            and focus.role == "entry"
            and focus.text == args.query
            and items
            and "selected" in items[0].states
            and count == (f"{len(items)} results" if len(items) != 1 else "1 result")
            and (items, count)
        )

    items, count = gate.wait(
        "the field holds the query, keeps the focus, and the count matches the list", typed
    )
    gate.transcript.append(f"bus: {count}; first {items[0].name!r} selected")
    gate.heard(f"the count ({count!r})", lambda s: re.search(rf"\b{re.escape(count)}\b", s))
    names = [item.name for item in items]
    if args.item not in names:
        raise GateFailed(f"{args.item!r} is not among the results: {names}")
    gate.note_speech()

    gate.step(f"3. arrow: the selection moves to {args.item!r}, and the focus with it")
    target = names.index(args.item)
    moves = target if target > 0 else 1
    for _ in range(moves):
        gate.keyboard.press("Down")
    if target == 0:
        gate.keyboard.press("Up")
    selected = gate.wait(
        f"{args.item!r} is selected and focused",
        lambda w, _s: (f := w and w.focused())
        and f.role == "list item"
        and f.name == args.item
        and "selected" in f.states
        and f,
    )
    gate.transcript.append(f"bus: focused list item {selected.name!r} [selected]")
    gate.heard(f"the selected result ({args.item!r})", lambda s: args.item in s)
    gate.note_speech()

    gate.step("4. Ctrl+B: the action panel, with its current action focused")
    gate.keyboard.press("Control_L", "b")
    action = gate.wait(
        "a menu of actions with the current one focused",
        lambda w, _s: w
        and (menu := w.find("menu", "Actions"))
        and menu.all("menu item")
        and (f := w.focused())
        and f.role == "menu item"
        and f,
    )
    menu_names = [item.name for item in launcher_window().find("menu", "Actions").all("menu item")]
    gate.transcript.append(f"bus: menu 'Actions' {menu_names}; focused {action.name!r}")
    gate.heard(f"the focused action ({action.name!r})", lambda s: action.name in s)
    gate.note_speech()

    gate.step(f"5. invoke: {action.name!r} pressed through AT-SPI's Action interface")
    acc = launcher_window().focused().acc
    actions = acc.get_action_iface()
    if actions is None or Atspi.Action.get_n_actions(actions) < 1:
        raise GateFailed(f"{action.name!r} offers no action to press")
    Atspi.Action.do_action(actions, 0)
    form = gate.wait(
        f"the {args.item!r} form, its first field focused",
        lambda w, _s: w
        and (form := w.find("form"))
        and (f := w.focused())
        and f.role in {"entry", "text", "password text"}
        and (form, f),
    )
    form_node, first = form
    fields = [f"{c.role} {c.name!r}" for c in form_node.children]
    gate.transcript.append(f"bus: form {form_node.name!r} {fields}; focused {first.name!r}")
    gate.heard(f"the first field ({first.name!r})", lambda s: first.name in s)
    gate.note_speech()

    gate.step(f"6. form: type into {first.name!r}, Tab to the checkbox, Space toggles it")
    gate.keyboard.type(args.field_text)
    gate.wait(
        f"{first.name!r} holds what was typed",
        lambda w, _s: (f := w and w.focused()) and f.name == first.name and f.text == args.field_text,
    )
    gate.transcript.append(f"bus: {first.name!r} text={args.field_text!r}")
    checkbox = None
    previous = first
    for _ in range(len(form_node.children)):
        gate.quiet()
        gate.speech.set_mark()
        gate.keyboard.press("Tab")
        focus = gate.wait(
            f"Tab moves the focus on from {previous.name!r}",
            lambda w, _s, before=previous: (f := w and w.focused())
            and f.role not in {"frame", "form"}
            and (f.role, f.name) != (before.role, before.name)
            and f,
        )
        gate.transcript.append(f"bus: focused {focus.role} {focus.name!r}")
        gate.heard(f"the focused control ({focus.name!r})", lambda s, n=focus.name: n in s)
        gate.note_speech()
        if focus.role == "check box":
            checkbox = focus
            break
        previous = focus
    if checkbox is None:
        raise GateFailed(f"Tab never reached a check box in {fields}")
    was = "checked" in checkbox.states
    gate.transcript.append(f"bus: check box {checkbox.name!r} checked={was}")
    gate.quiet()
    gate.speech.set_mark()
    gate.keyboard.press(" ")
    gate.wait(
        f"Space {'clears' if was else 'ticks'} {checkbox.name!r}",
        lambda w, _s: (f := w and w.focused())
        and f.role == "check box"
        and ("checked" in f.states) != was,
    )
    now = "not checked" if was else "checked"
    gate.transcript.append(f"bus: check box {checkbox.name!r} checked={not was}")
    gate.heard(f"the new state ({now!r})", lambda s: now in s.lower())
    gate.note_speech()

    gate.step("7. hide: Escape back to the search, clear it, and hide the launcher")
    gate.keyboard.press("Escape")
    gate.wait(
        "back at the search field",
        lambda w, _s: (f := w and w.focused()) and f.role == "entry" and f.name == SEARCH_NAME,
    )
    # Escape clears a query before it hides the window.
    for _ in range(3):
        gate.keyboard.press("Escape")
        time.sleep(1)
        if launcher_window() is None:
            break
    gate.wait("nothing of the window is left on the bus", lambda w, _s: w is None or None, 10)
    gate.transcript.append("bus: no launcher window")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--orca-log", required=True, help="Orca's debug log")
    parser.add_argument("--transcript", help="where to write what was checked and heard")
    parser.add_argument("--query", default="snippet")
    parser.add_argument("--item", default="Create Snippet", help="the result to arrow to")
    parser.add_argument("--field-text", default="orca gate")
    parser.add_argument("summon", nargs="+", help="the command that shows the launcher")
    args = parser.parse_args()

    gate = Gate(keyboard=Keyboard(), speech=Speech(Path(args.orca_log)))
    status_code = 0
    try:
        run(args, gate)
        print("\nORCA GATE PASSED", flush=True)
    except GateFailed as failure:
        print(f"\nORCA GATE FAILED: {failure}", file=sys.stderr, flush=True)
        status_code = 1
    finally:
        everything = Speech(Path(args.orca_log)).all()
        print("\n--- everything Orca said ---")
        for said in everything:
            print(f"  {said!r}")
        if args.transcript:
            lines = list(gate.transcript)
            lines += ["", "## everything Orca said", *(f"  {s!r}" for s in everything)]
            Path(args.transcript).write_text("\n".join(lines) + "\n")
    return status_code


if __name__ == "__main__":
    sys.exit(main())
