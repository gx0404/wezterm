"""The launcher's table of released config files must match the release commits in git."""
import re
import subprocess
import unittest

import gx_config_fingerprints as fingerprints

ENTRY = re.compile(r'^    \("([^"]+)", (\d+), 0x([0-9a-f]{4}(?:_[0-9a-f]{4}){3}), 0b([01]+)\),$', re.MULTILINE)


def committed() -> list[tuple[str, int, int, int]]:
    text = fingerprints.OUTPUT.read_text(encoding="utf-8")
    return [(path, int(length), int(value.replace("_", ""), 16), int(mask, 2))
            for path, length, value, mask in ENTRY.findall(text)]


class ReleasedConfigTests(unittest.TestCase):
    def test_fingerprint_matches_the_launcher(self):
        self.assertEqual(fingerprints.fnv1a(b""), 0xCBF29CE484222325)
        self.assertEqual(fingerprints.fnv1a(b"a"), 0xAF63DC4C8601EC8C)
        self.assertEqual(fingerprints.fnv1a(b"foobar"), 0x85944171F73967E8)
        self.assertEqual(fingerprints.fingerprint(b"a\r\nb\rc\r\n"), (6, fingerprints.fnv1a(b"a\nb\rc\n")))
        self.assertEqual(fingerprints.fingerprint(b"\0a\r\nb"), (5, fingerprints.fnv1a(b"\0a\r\nb")))

    def test_committed_table_is_generated_and_keeps_the_earlier_launch_upgrades(self):
        entries = committed()
        self.assertEqual(fingerprints.render(entries), fingerprints.OUTPUT.read_text(encoding="utf-8"))
        self.assertEqual(entries, sorted(set(entries)))
        self.assertIn(("config/launch.lua", 1935, 0x3A570763D22BE724, 0b01), entries)
        self.assertIn(("config/launch.lua", 2339, 0x7586010E9F4ED8DC, 0b10), entries)
        self.assertFalse({path for path, *_ in entries} & fingerprints.CONFIG_USER_DATA)

    def test_every_release_is_recomputed_from_git(self):
        entries = {entry[:3] for entry in committed()}
        releases = [release for release in fingerprints.RELEASES if fingerprints.available(release[1])]
        if not releases:
            self.skipTest("release commits are not in this clone")
        for release in releases:
            with self.subTest(release[0]):
                shipped = {entry[:3] for entry in fingerprints.released_entries([release])}
                self.assertGreater(len(shipped), 40)
                self.assertLessEqual(shipped, entries)
        if len(releases) == len(fingerprints.RELEASES):
            self.assertEqual(fingerprints.released_entries(), committed())

    def test_published_versions_are_the_dated_headings_before_the_newest(self):
        changelog = "## 0.3.0(TBD)\n## 0.2.0(2026-10-01)\n## 0.10.0(TBD)\n## 0.1.0(2026-09-29)\n"
        self.assertEqual(fingerprints.published_versions(changelog), ["0.2.0", "0.1.0"])
        self.assertEqual(fingerprints.published_versions("## 0.2.0(2026-10-01)\n## 0.1.0(2026-09-29)\n"),
                         ["0.1.0"])
        self.assertEqual(fingerprints.published_versions("# Changelog\n"), [])

    def test_a_published_release_missing_from_the_table_is_reported(self):
        changelog = "## 0.2.0(TBD)\n## 0.1.0(2026-09-29)\n"
        self.assertEqual(fingerprints.unlisted_releases(changelog), [])
        self.assertEqual(fingerprints.unlisted_releases(changelog, releases=()), ["GX Shell 0.1.0"])

    def test_every_published_gx_shell_release_is_listed_at_its_tag(self):
        if not fingerprints.CHANGELOG.is_file():
            self.skipTest("not inside the gx_shell monorepo")
        changelog = fingerprints.CHANGELOG.read_text(encoding="utf-8")
        self.assertEqual(fingerprints.unlisted_releases(changelog), [],
                         "append the release to gx_config_fingerprints.RELEASES and regenerate released.rs")
        commits = {name.split(" (")[0]: commit for name, commit, _ in fingerprints.RELEASES}
        for version in fingerprints.published_versions(changelog):
            tag = subprocess.run(["git", "-C", str(fingerprints.ROOT), "rev-parse", "-q", "--verify",
                                  f"gx-shell-v{version}^{{commit}}"], capture_output=True, text=True)
            if tag.returncode == 0:
                self.assertEqual(commits[f"GX Shell {version}"], tag.stdout.strip())


if __name__ == "__main__":
    unittest.main()
