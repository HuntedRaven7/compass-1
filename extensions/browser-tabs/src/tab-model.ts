//! Pure view helpers for the browser-tab list.
//!
//! Kept free of React and of `@vicinae/api` so the ordering, naming and
//! failure cases are testable with Node's own test runner. The runtime's
//! `Tab` also carries `browserId`; the published type leaves it out, so the
//! shape here names it as optional.

export interface BrowserTab {
	id: number;
	title?: string;
	url: string;
	active: boolean;
	browserId?: string;
	focus: () => void;
}

const BROWSER_NAMES: Record<string, string> = {
	firefox: "Firefox",
	"org.mozilla.firefox": "Firefox",
	chrome: "Chrome",
	chromium: "Chromium",
	brave: "Brave",
	edge: "Edge",
};

/** A stable name for a browser, even when the host reports an id we do not know. */
export function browserLabel(tab: BrowserTab): string {
	const id = (tab.browserId ?? "").trim();
	if (!id) return "Browser";
	if (BROWSER_NAMES[id]) return BROWSER_NAMES[id];
	const parts = id.split(/[./]/).filter(Boolean);
	const base = parts[parts.length - 1] ?? id;
	return base.charAt(0).toUpperCase() + base.slice(1);
}

/** What to show when a browser gives us no title, or an unparseable URL. */
export function tabTitle(tab: BrowserTab): string {
	const title = tab.title?.trim();
	if (title) return title;
	try {
		const host = new URL(tab.url).host;
		if (host) return host;
	} catch {
		// A tab URL need not be absolute; fall through to the placeholder.
	}
	return "Unnamed tab";
}

/** A total order so two browsers and duplicate titles never swap places. */
export function sortTabs(tabs: readonly BrowserTab[]): BrowserTab[] {
	return [...tabs].sort((a, b) => {
		const browser = browserLabel(a).localeCompare(browserLabel(b));
		if (browser !== 0) return browser;
		if (a.active !== b.active) return a.active ? -1 : 1;
		const title = tabTitle(a).localeCompare(tabTitle(b));
		if (title !== 0) return title;
		if (a.id !== b.id) return a.id - b.id;
		return a.url.localeCompare(b.url);
	});
}

/** One browser section. `id` is the identity; `label` is only its title. */
export interface BrowserGroup {
	id: string;
	label: string;
	tabs: BrowserTab[];
}

/**
 * Browser sections in a stable order. Grouping is by `browserId`, never by
 * the display label: two unknown browsers may end in the same word, and focus
 * still has to target the one the row came from.
 */
export function groupByBrowser(tabs: readonly BrowserTab[]): BrowserGroup[] {
	const groups = new Map<string, BrowserTab[]>();
	for (const tab of sortTabs(tabs)) {
		const id = tab.browserId?.trim() || "";
		const group = groups.get(id);
		if (group) group.push(tab);
		else groups.set(id, [tab]);
	}
	return [...groups.entries()]
		.map(([id, group]) => ({
			id,
			label: browserLabel(group[0]),
			tabs: group,
		}))
		.sort((a, b) => a.label.localeCompare(b.label) || a.id.localeCompare(b.id));
}

/** A focus failure must not tear down the extension's view. */
export function focusTab(tab: BrowserTab): boolean {
	try {
		tab.focus();
		return true;
	} catch {
		return false;
	}
}
