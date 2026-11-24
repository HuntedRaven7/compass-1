import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
	type BrowserTab,
	browserLabel,
	focusTab,
	groupByBrowser,
	sortTabs,
	tabTitle,
} from "../src/tab-model";

function tab(partial: Partial<BrowserTab> = {}): BrowserTab {
	return {
		id: 1,
		url: "https://example.com/page",
		active: false,
		focus: () => {},
		...partial,
	};
}

describe("tab naming", () => {
	it("uses a title, then the URL host, then a placeholder", () => {
		assert.equal(tabTitle(tab({ title: "  Inbox  " })), "Inbox");
		assert.equal(tabTitle(tab({ title: "  " })), "example.com");
		assert.equal(tabTitle(tab({ url: "about:blank" })), "Unnamed tab");
		assert.equal(tabTitle(tab({ url: "not a url" })), "Unnamed tab");
	});

	it("names a browser from its id and keeps unknown ones distinct", () => {
		assert.equal(browserLabel(tab({ browserId: "firefox" })), "Firefox");
		assert.equal(
			browserLabel(tab({ browserId: "org.mozilla.firefox" })),
			"Firefox",
		);
		assert.equal(browserLabel(tab({ browserId: "brave" })), "Brave");
		assert.equal(browserLabel(tab({ browserId: "" })), "Browser");
		assert.equal(
			browserLabel(tab({ browserId: "com.example.browser" })),
			"Browser",
		);
	});
});

describe("tab ordering", () => {
	it("is total: browser, active, title, id, then URL", () => {
		const input = [
			tab({ id: 4, browserId: "chrome", title: "Zeta", url: "https://z.dev" }),
			tab({
				id: 2,
				browserId: "firefox",
				title: "Alpha",
				url: "https://a.dev",
			}),
			tab({
				id: 1,
				browserId: "firefox",
				title: "Alpha",
				url: "https://b.dev",
			}),
			tab({ id: 9, browserId: "firefox", title: "Alpha", active: true }),
			tab({ id: 3, browserId: "chrome", title: "Alpha" }),
		];
		const sorted = sortTabs(input);
		assert.deepEqual(
			sorted.map((t) => `${browserLabel(t)}:${t.id}`),
			["Chrome:3", "Chrome:4", "Firefox:9", "Firefox:1", "Firefox:2"],
		);
		assert.equal(input[0]?.id, 4, "sorting does not mutate the input");
	});

	it("groups browsers deterministically", () => {
		const input = [
			tab({ id: 2, browserId: "chrome", title: "B" }),
			tab({ id: 1, browserId: "firefox", title: "A" }),
			tab({ id: 3, browserId: "chrome", title: "A" }),
		];
		const groups = groupByBrowser(input);
		assert.deepEqual(
			groups.map(([label, tabs]) => [label, tabs.map((t) => t.id)]),
			[
				["Chrome", [3, 2]],
				["Firefox", [1]],
			],
		);
	});

	it("keeps duplicate titles as separate rows", () => {
		const groups = groupByBrowser([
			tab({ id: 1, title: "Mail" }),
			tab({ id: 2, title: "Mail" }),
		]);
		assert.equal(groups[0]?.[1].length, 2);
	});

	it("an empty list has no groups", () => {
		assert.deepEqual(groupByBrowser([]), []);
	});
});

describe("focus", () => {
	it("calls focus once and reports success", () => {
		let calls = 0;
		assert.equal(
			focusTab(
				tab({
					focus: () => {
						calls++;
					},
				}),
			),
			true,
		);
		assert.equal(calls, 1);
	});

	it("a focus error does not escape into the view", () => {
		assert.equal(
			focusTab(
				tab({
					focus: () => {
						throw new Error("browser closed");
					},
				}),
			),
			false,
		);
	});
});
