"""Tests for check_schema_impact.py. Run: python3 -m unittest discover -s scripts -p 'test_*.py'"""
import subprocess
import tempfile
import unittest
from pathlib import Path

from check_schema_impact import offending_commits

LIB = """#![no_std]
use soroban_sdk::contracttype;

#[contracttype]
#[derive(Clone)]
pub struct Attestation {
    pub issuer: u32,
    pub subject: u32,
}

#[contracttype]
pub enum DataKey {
    Admin,
}

pub fn helper() -> u32 {
    1
}
"""


class SchemaImpactTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self._tmp.name)
        self.git("init", "-q")
        self.git("config", "user.email", "t@example.com")
        self.git("config", "user.name", "t")
        self.commit("src/lib.rs", LIB, "chore: initial")
        self.base = self.git("rev-parse", "HEAD").strip()

    def tearDown(self):
        self._tmp.cleanup()

    def git(self, *args):
        return subprocess.run(
            ["git", "-C", str(self.repo), *args], check=True, capture_output=True, text=True
        ).stdout

    def commit(self, path, content, message):
        file = self.repo / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(content)
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)
        return self.git("rev-parse", "HEAD").strip()

    def edit(self, old, new, message, path="src/lib.rs"):
        content = (self.repo / path).read_text().replace(old, new)
        return self.commit(path, content, message)

    def check(self):
        return offending_commits(self.repo, f"{self.base}..HEAD")

    def test_new_datakey_variant_without_trailer_fails(self):
        sha = self.edit("    Admin,\n", "    Admin,\n    Paused,\n", "feat: pause")
        self.assertEqual(self.check(), [sha])

    def test_trailer_satisfies_check(self):
        self.edit(
            "    Admin,\n", "    Admin,\n    Paused,\n",
            "feat: pause\n\nSchema-Impact: additive, no migration",
        )
        self.assertEqual(self.check(), [])

    def test_field_inside_contracttype_struct_fails(self):
        sha = self.edit("    pub subject: u32,\n", "    pub subject: u32,\n    pub ttl: u64,\n", "feat: ttl")
        self.assertEqual(self.check(), [sha])

    def test_unrelated_change_passes(self):
        self.edit("    1\n", "    2\n", "fix: helper")
        self.assertEqual(self.check(), [])

    def test_comment_only_change_passes(self):
        self.edit("    pub issuer: u32,\n", "    /// Who issued it.\n    pub issuer: u32,\n", "docs: issuer")
        self.assertEqual(self.check(), [])

    def test_test_files_are_ignored(self):
        self.commit("src/test.rs", "fn t() { let _ = DataKey::Admin; }\n", "test: datakey")
        self.assertEqual(self.check(), [])


if __name__ == "__main__":
    unittest.main()
