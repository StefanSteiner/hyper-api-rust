"""Unit tests for the release-PR version-forward guard.

Run: python3 -m unittest .github/scripts/test_verify_release_pr_version.py
(or: cd .github/scripts && python3 -m unittest test_verify_release_pr_version)
"""
import importlib.util
import unittest
from pathlib import Path

# Load the hyphenated module file by path (not importable as a normal name).
_spec = importlib.util.spec_from_file_location(
    "verify_release_pr_version",
    Path(__file__).with_name("verify-release-pr-version.py"),
)
mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(mod)
compare = mod.compare


class TestCompare(unittest.TestCase):
    def test_rc_forward(self):
        self.assertEqual(compare("1.0.0-rc.3", "1.0.0-rc.2"), 1)  # base>head

    def test_rc_backward_is_head_less(self):
        # head rc.2 vs base rc.3 -> head is behind
        self.assertEqual(compare("1.0.0-rc.2", "1.0.0-rc.3"), -1)

    def test_rc_numeric_not_lexical(self):
        # rc.10 must sort ABOVE rc.9 (numeric identifiers)
        self.assertEqual(compare("1.0.0-rc.10", "1.0.0-rc.9"), 1)

    def test_release_beats_prerelease(self):
        # 1.0.0 (no prerelease) > 1.0.0-rc.5
        self.assertEqual(compare("1.0.0", "1.0.0-rc.5"), 1)

    def test_prerelease_below_release(self):
        # 1.0.0-rc.2 < 1.0.0  (the post-graduation backward landmine)
        self.assertEqual(compare("1.0.0-rc.2", "1.0.0"), -1)

    def test_equal(self):
        self.assertEqual(compare("1.0.0-rc.3", "1.0.0-rc.3"), 0)

    def test_triple_bump(self):
        self.assertEqual(compare("1.0.1", "1.0.0"), 1)
        self.assertEqual(compare("2.0.0", "1.9.9"), 1)

    def test_build_metadata_ignored(self):
        self.assertEqual(compare("1.0.0+abc", "1.0.0+xyz"), 0)

    def test_numeric_identifier_below_alphanumeric(self):
        # semver 2.0: a numeric prerelease identifier has LOWER precedence than
        # an alphanumeric one, so 1.0.0-9 < 1.0.0-a.
        self.assertEqual(compare("1.0.0-9", "1.0.0-a"), -1)
        self.assertEqual(compare("1.0.0-a", "1.0.0-9"), 1)

    def test_longer_identifier_list_wins(self):
        # semver 2.0: when the shared prefix is equal, the LONGER identifier
        # list has higher precedence, so 1.0.0-rc.1.1 > 1.0.0-rc.1.
        self.assertEqual(compare("1.0.0-rc.1.1", "1.0.0-rc.1"), 1)
        self.assertEqual(compare("1.0.0-rc.1", "1.0.0-rc.1.1"), -1)


class TestForwardRule(unittest.TestCase):
    def test_is_forward(self):
        # head >= base -> not backward
        self.assertTrue(mod.is_forward_or_equal("1.0.0-rc.3", "1.0.0-rc.4"))
        self.assertTrue(mod.is_forward_or_equal("1.0.0-rc.3", "1.0.0-rc.3"))
        self.assertTrue(mod.is_forward_or_equal("1.0.0-rc.5", "1.0.0"))

    def test_is_backward(self):
        self.assertFalse(mod.is_forward_or_equal("1.0.0-rc.3", "1.0.0-rc.2"))
        self.assertFalse(mod.is_forward_or_equal("1.0.0", "1.0.0-rc.2"))


if __name__ == "__main__":
    unittest.main()
