// System Health
//
// Failed systemd units, the system's and your own, and which update tools this
// system has: bootc, rpm-ostree and flatpak. Enter copies a command to look
// further; Ctrl+R checks again.
//
// Shows: `process::run` with a timeout on every call and a budget over all of
// them; telling "not installed" (`()`), "timed out" and "failed" apart; saying
// "unsupported" plainly on a system without systemd; and `filtering: true`, so
// the launcher filters the rows itself and the programs run only when the
// script opens or is refreshed, never per keystroke and never in the
// background.

const BUDGET_MS = 1500;  // every program together; a call may take 2 s in all
const SYSTEMD_MS = 500;
const TOOL_MS = 250;

const TOOLS = [
    #{ program: "bootc", purpose: "Updates the operating system image", command: "sudo bootc upgrade" },
    #{ program: "rpm-ostree", purpose: "Updates the operating system image", command: "rpm-ostree upgrade" },
    #{ program: "flatpak", purpose: "Updates Flatpak applications", command: "flatpak update" },
];

// Runs a program within what is left of the budget and says how it went, as
// `state`: "missing", "skipped", "timed_out", "failed" or "ok".
fn probe(program, args, wanted, deadline) {
    let left = deadline - time::now_ms();
    if left <= 0 {
        return #{ state: "skipped", out: "", err: "" };
    }
    let result = process::run(program, args, if wanted < left { wanted } else { left });
    if type_of(result) == "()" {
        return #{ state: "missing", out: "", err: "" };
    }
    if result.timed_out {
        return #{ state: "timed_out", out: "", err: "" };
    }
    let state = if result.code == 0 { "ok" } else { "failed" };
    #{ state: state, out: result.stdout, err: result.stderr }
}

fn first_line(text) {
    for line in text.split("\n") {
        let l = line;
        l.trim();
        if l != "" { return l; }
    }
    ""
}

// The first word with a digit in it: "bootc 1.1.4" and rpm-ostree's
// "Version: '2024.7'" both give the version.
fn version(text) {
    for word in text.split() {
        if word.to_chars().some(|c| c >= '0' && c <= '9') {
            let v = word;
            v.replace("'", "");
            return v;
        }
    }
    ()
}

// One row standing in for something that could not be listed.
fn notice(id, title, subtitle, icon, tag, color) {
    #{ id: id, title: title, subtitle: subtitle, icon: icon, accessory: #{ tag: tag, color: color } }
}

fn timed_out(id, what) {
    notice(id, `${what} did not answer`, "It did not reply in time. Press Ctrl+R to try again.",
           "hourglass", "Timed out", "orange")
}

fn skipped(id) {
    notice(id, "Not checked", "The time for checking ran out first. Press Ctrl+R to try again.",
           "hourglass", "Not checked", "orange")
}

// Whether systemd runs this system: `#{ supported, state }`, or
// `#{ supported: false, row }` saying why not.
fn manager(deadline) {
    let run = probe("systemctl", ["is-system-running"], global::SYSTEMD_MS, deadline);
    let state = first_line(run.out);
    // `is-system-running` exits non-zero for any state but "running", so its
    // answer is what it printed.
    if run.state == "missing" {
        return #{ supported: false, row: notice("systemd", "systemd is unsupported here",
            "systemctl is not installed, so this system does not use systemd.",
            "circle-disabled", "Unsupported", ()) };
    }
    if state == "offline" {
        return #{ supported: false, row: notice("systemd", "systemd is unsupported here",
            "This system was not started by systemd, so it has no units to check.",
            "circle-disabled", "Unsupported", ()) };
    }
    if run.state == "timed_out" {
        return #{ supported: false, row: timed_out("systemd", "systemd") };
    }
    if run.state == "skipped" {
        return #{ supported: false, row: skipped("systemd") };
    }
    if ["initializing", "starting", "running", "degraded", "maintenance", "stopping"].contains(state) {
        return #{ supported: true, state: state };
    }
    let reason = first_line(run.err);
    #{ supported: false, row: notice("systemd", "Compass could not ask systemd",
        if reason == "" { `systemctl answered "${state}".` } else { reason },
        "exclamationmark", "Error", "red") }
}

// The failed units of one manager: the system's (`flag` is "") or the
// user's ("--user").
fn failed_units(scope, flag, deadline) {
    let args = if flag == "" { [] } else { [flag] };
    args += ["list-units", "--failed", "--plain", "--no-legend", "--no-pager"];
    let run = probe("systemctl", args, global::SYSTEMD_MS, deadline);
    let id = `${scope}:status`;
    if run.state == "timed_out" { return [timed_out(id, "systemd")]; }
    if run.state == "skipped" { return [skipped(id)]; }
    if run.state != "ok" {
        let reason = first_line(run.err);
        return [notice(id, `Compass could not list the ${scope} units`,
            if reason == "" { "systemctl failed without saying why." } else { reason },
            "exclamationmark", "Error", "red")];
    }

    let command = if flag == "" { "systemctl" } else { `systemctl ${flag}` };
    let rows = [];
    for line in run.out.split("\n") {
        let fields = line.split();
        if fields.len() < 4 { continue; }
        // UNIT LOAD ACTIVE SUB DESCRIPTION; the description keeps its spaces.
        let description = line;
        description.trim();
        for i in 0..4 {
            description = description.sub_string(fields[i].len());
            description.trim();
        }
        let unit = fields[0];
        rows.push(#{
            id: `${scope}:${unit}`,
            title: unit,
            subtitle: description,
            icon: "warning",
            keywords: [scope],
            accessory: #{ tag: "Failed", color: "red" },
            actions: [
                #{ title: "Copy Status Command", copy: `${command} status ${unit}` },
                #{ title: "Copy Unit Name", copy: unit },
            ],
        });
    }
    if rows.is_empty() {
        rows.push(notice(id, "No failed units", `Every ${scope} unit is running or stopped cleanly.`,
                         "check-circle", "OK", "green"));
    }
    rows
}

fn updater(tool, deadline) {
    let run = probe(tool.program, ["--version"], global::TOOL_MS, deadline);
    let row = #{ id: `update:${tool.program}`, title: tool.program, subtitle: tool.purpose, icon: "download" };
    if run.state == "ok" {
        let v = version(run.out);
        row.accessories = if v == () { [] } else { [v] };
        row.accessories.push(#{ tag: "Available", color: "green" });
        row.actions = [#{ title: "Copy Update Command", copy: tool.command }];
        return row;
    }
    row.icon = "minus-circle";
    row.accessory = if run.state == "missing" {
        #{ tag: "Not installed" }
    } else if run.state == "timed_out" {
        #{ tag: "Timed out", color: "orange" }
    } else if run.state == "skipped" {
        #{ tag: "Not checked", color: "orange" }
    } else {
        // Inside a Flatpak a program missing on the host lands here, as
        // `flatpak-spawn` failing to start it.
        #{ tag: "Unavailable", tooltip: first_line(run.err) }
    };
    row
}

fn refresh() {
    #{ rerender: true }
}

fn search(query) {
    let deadline = time::now_ms() + global::BUDGET_MS;
    let sections = [];
    let systemd = manager(deadline);
    if systemd.supported {
        sections.push(#{ id: "system", title: "Failed System Units", subtitle: `State: ${systemd.state}`,
                         items: failed_units("system", "", deadline) });
        sections.push(#{ id: "user", title: "Failed User Units", items: failed_units("user", "--user", deadline) });
    } else {
        sections.push(#{ id: "systemd", title: "systemd", items: [systemd.row] });
    }
    let updates = [];
    for tool in global::TOOLS { updates.push(updater(tool, deadline)); }
    sections.push(#{ id: "updates", title: "Updates", items: updates });

    #{
        sections: sections,
        filtering: true,
        placeholder: "Filter units and update tools",
        actions: [#{ title: "Refresh", run: "refresh", icon: "arrow-clockwise", shortcut: "ctrl+r" }],
    }
}
