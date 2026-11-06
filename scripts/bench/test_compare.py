import tempfile
import unittest

import compare

UPSTREAM = {"repository": "vicinaehq/vicinae", "tag": "v0.29.0", "commit": "c3415a3", "sha256": "4d32f758"}
WORKLOAD = {"network": "host", "corpus": "c0ffee"}


def summary(**overrides):
    figures = {"ping_ns": 20_000, "ready_ms": 90.0, "first_frame_ms": 800.0, "complete_ms": 900.0,
               "type_ms": 50.0, "idle_rss_kib": 250_000, "idle_pss_kib": 220_000}
    return figures | overrides


class UpstreamGateTests(unittest.TestCase):
    def test_every_gated_action_pairs_both_engines_from_one_run(self):
        section, points = compare.gate({"compass": summary(), "upstream": summary(ping_ns=40_000)},
                                       UPSTREAM, WORKLOAD)
        self.assertEqual([point["metric"] for point in points], [metric for metric, _, _ in compare.GATED])
        ping = points[0]
        self.assertEqual((ping["value"], ping["upstream"]["value"]), (20_000, 40_000))
        self.assertEqual(ping["upstream"]["sha256"], "4d32f758")
        self.assertEqual(ping["env"], {"profile": "release", "renderer": compare.RENDERER, **WORKLOAD})
        self.assertEqual(section["actions"][0], {"metric": "versus.ping", "unit": "ns",
                                                 "compass": 20_000, "upstream": 40_000})

    def test_a_probe_that_differs_between_engines_is_excluded_not_gated(self):
        gated = {key for _, key, _ in compare.GATED}
        self.assertNotIn("ready_ms", gated)
        section, _ = compare.gate({"compass": summary(), "upstream": summary()}, UPSTREAM, WORKLOAD)
        excluded = {entry["comparison"]: entry["reason"] for entry in section["excluded"]}
        self.assertIn("engine ready (ready_ms)", excluded)
        self.assertIn("search over IPC", excluded)
        self.assertTrue(all(reason.endswith(".") for reason in excluded.values()))

    def test_a_missing_figure_is_an_error_not_a_skipped_action(self):
        broken = summary()
        del broken["type_ms"]
        with self.assertRaises(KeyError):
            compare.gate({"compass": broken, "upstream": summary()}, UPSTREAM, WORKLOAD)


class EngineLogTests(unittest.TestCase):
    def test_an_exit_error_quotes_the_end_of_the_engine_log(self):
        with tempfile.NamedTemporaryFile("w", suffix=".log") as log:
            log.write("starting\n")
            log.write("vicinae-server: error while loading shared libraries: libOpenGL.so.0\n")
            quoted = compare.tail(log, lines=1)
        self.assertEqual(quoted, "\n  | vicinae-server: error while loading shared libraries: libOpenGL.so.0")


if __name__ == "__main__":
    unittest.main()
