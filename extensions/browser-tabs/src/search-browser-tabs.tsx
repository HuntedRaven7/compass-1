import {
	Action,
	ActionPanel,
	BrowserExtension,
	Icon,
	List,
} from "@vicinae/api";
import { useEffect, useState } from "react";
import {
	type BrowserTab,
	focusTab,
	groupByBrowser,
	tabTitle,
} from "./tab-model";

export default function SearchBrowserTabs() {
	const [tabs, setTabs] = useState<BrowserTab[]>([]);
	const [loaded, setLoaded] = useState(false);
	const [error, setError] = useState<string>();

	useEffect(() => {
		let active = true;
		BrowserExtension.getTabs()
			.then((all: BrowserExtension.Tab[]) => {
				if (!active) return;
				// The runtime adds browserId to every tab; the published type
				// omits it, so treat it as optional rather than casting blindly.
				setTabs(all as BrowserTab[]);
				setLoaded(true);
			})
			.catch((reason: unknown) => {
				if (!active) return;
				setError(String(reason));
				setLoaded(true);
			});
		return () => {
			active = false;
		};
	}, []);

	if (!loaded) {
		return <List isLoading searchBarPlaceholder="Search browser tabs..." />;
	}
	if (error) {
		return (
			<List searchBarPlaceholder="Search browser tabs...">
				<List.EmptyView
					title="Browser bridge unavailable"
					description={`The launcher could not read open tabs: ${error}`}
					icon={Icon.Browserstack}
				/>
			</List>
		);
	}
	if (tabs.length === 0) {
		return (
			<List searchBarPlaceholder="Search browser tabs...">
				<List.EmptyView
					title="No open tabs"
					description="Connect Firefox or a supported Chromium browser, then open a page."
					icon={Icon.Browserstack}
				/>
			</List>
		);
	}

	return (
		<List searchBarPlaceholder="Search browser tabs...">
			{groupByBrowser(tabs).map((group) => (
				<List.Section key={group.id} title={group.label}>
					{group.tabs.map((tab) => (
						<List.Item
							key={`${tab.browserId ?? "browser"}:${tab.id}`}
							id={`${tab.browserId ?? "browser"}:${tab.id}`}
							title={tabTitle(tab)}
							subtitle={tab.url}
							icon={Icon.Globe01}
							accessories={tab.active ? [{ text: "Active" }] : undefined}
							actions={
								<ActionPanel>
									<Action
										title="Focus Tab"
										onAction={() => {
											focusTab(tab);
										}}
									/>
								</ActionPanel>
							}
						/>
					))}
				</List.Section>
			))}
		</List>
	);
}
