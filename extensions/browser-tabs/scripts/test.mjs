// Bundles the extension's pure-model tests with the esbuild the SDK already
// installs, then runs the bundles with Node's own test runner. No test
// framework and nothing an offline build would have to fetch.
import * as esbuild from "esbuild";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const root = join(import.meta.dirname, "..");
const testDir = join(root, "test");
const entryPoints = readdirSync(testDir)
	.filter((name) => name.endsWith(".test.ts"))
	.map((name) => join(testDir, name));

const outdir = mkdtempSync(join(tmpdir(), "browser-tabs-tests-"));
let status = 1;
try {
	await esbuild.build({
		entryPoints,
		bundle: true,
		outdir,
		format: "cjs",
		platform: "node",
		logLevel: "warning",
	});
	const files = readdirSync(outdir)
		.filter((name) => name.endsWith(".js"))
		.map((name) => join(outdir, name));
	status =
		spawnSync(process.execPath, ["--test", ...files], { stdio: "inherit" })
			.status ?? 1;
} finally {
	rmSync(outdir, { recursive: true, force: true });
}
process.exit(status);
