import { getClient } from "./client";

/**
 * Use browser extension features, provided that the user has it installed and active.
 */
export namespace BrowserExtension {
	export type Tab = {
		id: number;
		title?: string;
		url: string;
		active: boolean;
		focus: () => void;
	};

	export async function getTabs(): Promise<Tab[]> {
		const browser = getClient().BrowserExtension;
		return browser.getTabs().then((bb) =>
			bb.map<Tab>((b) => ({
				...b,
				focus: () => {
					// The API keeps `focus` synchronous, so a rejected request
					// would otherwise be an unhandled rejection and take down
					// the worker when the browser has gone away.
					void getClient().BrowserExtension.focusTab(b.browserId, b.id).catch(
						(error: unknown) => {
							console.error("BrowserExtension.focusTab failed", error);
						},
					);
				},
			})),
		);
	}
}
