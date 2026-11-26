// Compass <-> GNOME Shell helper extension.
//
// Implements the versioned contract in ./dbus (the same files as
// crates/compass-shell/dbus, which a test keeps identical): windows (and
// their workspaces) and the clipboard, the things Mutter gives no other
// application a way to reach.
// Deliberately small, so a GNOME release breaks this file rather than the
// launcher. Anything not in the contract does not belong here.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const CONTRACT_VERSION = 4;
const WINDOWS_PATH = '/org/tunaos/compass/Shell/Windows';
const CLIPBOARD_PATH = '/org/tunaos/compass/Shell/Clipboard';

// Bursts of window events (a window opening fires several) become one signal.
const COALESCE_MS = 100;

// Preferred first. A selection's first match here is the one recorded.
const CLIPBOARD_MIME_PREFERENCE = [
    'text/uri-list',
    'image/png',
    'image/jpeg',
    'text/plain;charset=utf-8',
    'UTF8_STRING',
    'text/plain',
];

// Paste waits this long for focus to leave the launcher, then gives up, and
// presses the shortcut this long after focus lands so the window is ready.
const PASTE_FOCUS_TIMEOUT_MS = 2000;
const PASTE_SETTLE_MS = 30;

// Password managers mark secrets with this; the contract says such content
// is never emitted.
const CONCEALED_MIME = 'x-kde-passwordManagerHint';

function readInterfaceXml(extension, name) {
    const file = extension.dir.get_child('dbus').get_child(`${name}.xml`);
    const [, bytes] = file.load_contents(null);
    return new TextDecoder().decode(bytes);
}

class WindowsService {
    constructor(xml) {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(xml, this);
        this._signals = [];
        this._pending = 0;
    }

    get Version() {
        return CONTRACT_VERSION;
    }

    ListWindows() {
        const windows = global.display.get_tab_list(Meta.TabList.NORMAL_ALL, null);
        return windows.map(window => {
            const workspace = window.get_workspace();
            const entry = {
                id: new GLib.Variant('u', window.get_stable_sequence()),
                title: new GLib.Variant('s', window.get_title() ?? ''),
                wm_class: new GLib.Variant('s', window.get_wm_class() ?? ''),
                wm_class_instance: new GLib.Variant('s', window.get_wm_class_instance() ?? ''),
                focused: new GLib.Variant('b', window.has_focus()),
                workspace: new GLib.Variant('i', workspace ? workspace.index() : -1),
                can_close: new GLib.Variant('b', window.can_close()),
            };
            const pid = window.get_pid();
            if (pid > 0)
                entry.pid = new GLib.Variant('u', pid);
            entry.fullscreen = new GLib.Variant('b', window.is_fullscreen());
            const frame = window.get_frame_rect();
            entry.x = new GLib.Variant('i', frame.x);
            entry.y = new GLib.Variant('i', frame.y);
            entry.width = new GLib.Variant('i', frame.width);
            entry.height = new GLib.Variant('i', frame.height);
            return entry;
        });
    }

    ActivateWindow(id) {
        const window = this._find(id);
        if (window)
            Main.activateWindow(window);
    }

    CloseWindow(id) {
        const window = this._find(id);
        if (window && window.can_close())
            window.delete(global.get_current_time());
    }

    // Contract 4. The name is what GNOME's preferences call it: the
    // `workspace-names` setting, else Mutter's own "Workspace N".
    ListWorkspaces() {
        const manager = global.workspace_manager;
        const active = manager.get_active_workspace_index();
        const workspaces = [];
        for (let index = 0; index < manager.get_n_workspaces(); index++) {
            const workspace = manager.get_workspace_by_index(index);
            const fullscreen = workspace
                ? workspace.list_windows().some(window => window.is_fullscreen())
                : false;
            workspaces.push({
                index: new GLib.Variant('i', index),
                name: new GLib.Variant('s', Meta.prefs_get_workspace_name(index) ?? ''),
                active: new GLib.Variant('b', index === active),
                has_fullscreen: new GLib.Variant('b', fullscreen),
            });
        }
        return workspaces;
    }

    ActivateWorkspace(index) {
        const manager = global.workspace_manager;
        if (index < 0 || index >= manager.get_n_workspaces())
            return;
        manager.get_workspace_by_index(index)?.activate(global.get_current_time());
    }

    _find(id) {
        return global.display
            .get_tab_list(Meta.TabList.NORMAL_ALL, null)
            .find(window => window.get_stable_sequence() === id) ?? null;
    }

    enable() {
        this._dbus.export(Gio.DBus.session, WINDOWS_PATH);
        const changed = () => this._changed();
        const display = global.display;
        this._signals.push([display, display.connect('window-created', (_d, window) => {
            this._watch(window);
            changed();
        })]);
        this._signals.push([display, display.connect('notify::focus-window', changed)]);
        const manager = global.workspace_manager;
        for (const signal of ['active-workspace-changed', 'workspace-added', 'workspace-removed'])
            this._signals.push([manager, manager.connect(signal, changed)]);
        for (const window of display.get_tab_list(Meta.TabList.NORMAL_ALL, null))
            this._watch(window);
    }

    _watch(window) {
        const changed = () => this._changed();
        this._signals.push([window, window.connect('unmanaged', changed)]);
        this._signals.push([window, window.connect('notify::title', changed)]);
        this._signals.push([window, window.connect('workspace-changed', changed)]);
    }

    _changed() {
        if (this._pending)
            return;
        this._pending = GLib.timeout_add(GLib.PRIORITY_DEFAULT, COALESCE_MS, () => {
            this._pending = 0;
            this._dbus.emit_signal('WindowsChanged', null);
            return GLib.SOURCE_REMOVE;
        });
    }

    disable() {
        for (const [object, id] of this._signals)
            object.disconnect(id);
        this._signals = [];
        if (this._pending) {
            GLib.source_remove(this._pending);
            this._pending = 0;
        }
        this._dbus.unexport();
    }
}

class ClipboardService {
    constructor(xml) {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(xml, this);
        this._clipboard = St.Clipboard.get_default();
        this._selection = global.display.get_selection();
        this._ownerChanged = 0;
        this._pasteFocus = 0;
        this._pasteTimeout = 0;
        this._pasteSettle = 0;
        this._keyboard = null;
    }

    get Version() {
        return CONTRACT_VERSION;
    }

    GetClipboardAsync(_params, invocation) {
        this._read((bytes, mime) => {
            invocation.return_value(new GLib.Variant('(ays)', [bytes, mime]));
        });
    }

    GetPrimarySelectionAsync(_params, invocation) {
        this._clipboard.get_text(St.ClipboardType.PRIMARY, (_clipboard, text) => {
            invocation.return_value(new GLib.Variant('(s)', [text ?? '']));
        });
    }

    SetClipboard(content, mimeType) {
        this._clipboard.set_content(
            St.ClipboardType.CLIPBOARD, mimeType, new GLib.Bytes(content));
    }

    // Arms a paste into whichever window focus moves to next, and returns.
    // Called while the launcher is still focused; the launcher then hides.
    Paste(shiftWmClasses) {
        this._cancelPaste();
        const from = global.display.focus_window;
        const shift = new Set(shiftWmClasses);

        this._pasteFocus = global.display.connect('notify::focus-window', () => {
            const target = global.display.focus_window;
            if (!target || target === from)
                return;
            this._cancelPaste();
            this._pasteSettle = GLib.timeout_add(GLib.PRIORITY_DEFAULT, PASTE_SETTLE_MS, () => {
                this._pasteSettle = 0;
                if (global.display.focus_window === target)
                    this._pressPaste(shift.has((target.get_wm_class() ?? '').toLowerCase()));
                return GLib.SOURCE_REMOVE;
            });
        });
        this._pasteTimeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, PASTE_FOCUS_TIMEOUT_MS, () => {
            this._pasteTimeout = 0;
            this._cancelPaste();
            return GLib.SOURCE_REMOVE;
        });
    }

    _pressPaste(withShift) {
        // GNOME 51 removed the default-backend getter; the stage's context
        // carries the backend there. Older Shells have no get_context().
        const backend = global.stage.get_context
            ? global.stage.get_context().get_backend()
            : Clutter.get_default_backend();
        this._keyboard ??= backend.get_default_seat()
            .create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        const modifiers = withShift
            ? [Clutter.KEY_Control_L, Clutter.KEY_Shift_L]
            : [Clutter.KEY_Control_L];
        const time = GLib.get_monotonic_time();
        for (const key of modifiers)
            this._keyboard.notify_keyval(time, key, Clutter.KeyState.PRESSED);
        this._keyboard.notify_keyval(time, Clutter.KEY_v, Clutter.KeyState.PRESSED);
        this._keyboard.notify_keyval(time, Clutter.KEY_v, Clutter.KeyState.RELEASED);
        for (const key of modifiers.reverse())
            this._keyboard.notify_keyval(time, key, Clutter.KeyState.RELEASED);
    }

    _cancelPaste() {
        if (this._pasteFocus) {
            global.display.disconnect(this._pasteFocus);
            this._pasteFocus = 0;
        }
        if (this._pasteTimeout) {
            GLib.source_remove(this._pasteTimeout);
            this._pasteTimeout = 0;
        }
        if (this._pasteSettle) {
            GLib.source_remove(this._pasteSettle);
            this._pasteSettle = 0;
        }
    }

    // Reads the current selection in the preferred type: `(bytes, mime)`, or
    // empty bytes and an empty mime when there is nothing to offer, or when
    // the owner marked it concealed.
    _read(done) {
        const offered = this._clipboard.get_mimetypes(St.ClipboardType.CLIPBOARD);
        if (offered.includes(CONCEALED_MIME)) {
            done(new Uint8Array(), '');
            return;
        }
        const mime = CLIPBOARD_MIME_PREFERENCE.find(type => offered.includes(type));
        if (!mime) {
            done(new Uint8Array(), '');
            return;
        }
        this._clipboard.get_content(St.ClipboardType.CLIPBOARD, mime, (_clipboard, bytes) => {
            const data = bytes ? bytes.get_data() ?? new Uint8Array() : new Uint8Array();
            done(data, data.length ? mime : '');
        });
    }

    enable() {
        this._dbus.export(Gio.DBus.session, CLIPBOARD_PATH);
        this._ownerChanged = this._selection.connect('owner-changed', (_sel, type) => {
            if (type !== Meta.SelectionType.SELECTION_CLIPBOARD)
                return;
            const app = Shell.WindowTracker.get_default().focus_app;
            const source = app?.get_id() ?? '';
            this._read((bytes, mime) => {
                if (!mime)
                    return;
                this._dbus.emit_signal(
                    'ClipboardChanged', new GLib.Variant('(ayss)', [bytes, mime, source]));
            });
        });
    }

    disable() {
        this._cancelPaste();
        this._keyboard = null;
        if (this._ownerChanged) {
            this._selection.disconnect(this._ownerChanged);
            this._ownerChanged = 0;
        }
        this._dbus.unexport();
    }
}

export default class CompassExtension extends Extension {
    enable() {
        this._windows = new WindowsService(
            readInterfaceXml(this, 'org.tunaos.compass.Shell.Windows'));
        this._clipboard = new ClipboardService(
            readInterfaceXml(this, 'org.tunaos.compass.Shell.Clipboard'));
        this._windows.enable();
        this._clipboard.enable();
    }

    disable() {
        this._clipboard?.disable();
        this._windows?.disable();
        this._clipboard = null;
        this._windows = null;
    }
}
