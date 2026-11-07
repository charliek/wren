"""Local fixture coverage for the bounded collector, not a legal-license audit."""

from contextlib import redirect_stderr, redirect_stdout
import copy
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools import dependency_notices as notices


MIT_TEXT = """MIT License

Copyright (c) 2026 Fixture Authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"""

BSD_TEXT = """BSD 3-Clause License

Copyright (c) 2026, Fixture Contributors
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice,
   this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.
3. Neither the name of the copyright holder nor the names of its contributors
   may be used to endorse or promote products derived from this software
   without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
"""


class DependencyNoticesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="wren-notices-", dir=Path(tempfile.gettempdir()).resolve())
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.packages = []
        # Workspace entries intentionally have no license, manifest, or source:
        # excluding by workspace_members must happen before third-party parsing.
        self.own = {"id": "private-workspace-id", "name": "wren-iced", "license": None}

    def crate(self, name="fixture", *, version="1.2.3", license="MIT", files=None, **extra):
        root = self.base / "private cache" / f"{name}-{version}"
        root.mkdir(parents=True)
        (root / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "{version}"\n')
        for relative, content in (files or {}).items():
            target = root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content.encode("utf-8") if isinstance(content, str) else content)
        package = {"id": f"private-package-id:{name}:{version}", "name": name, "version": version,
                   "license": license, "manifest_path": str(root / "Cargo.toml"), **extra}
        self.packages.append(package)
        return root, package

    def metadata(self):
        return {"workspace_members": [self.own["id"]], "packages": [self.own, *self.packages]}

    def report(self):
        return notices.notices_from_metadata(self.metadata())

    def test_full_mit_bsd_copyright_and_notice_files_are_copied_without_substitution(self):
        self.crate(license="MIT OR BSD-3-Clause", files={
            "LICENSE-MIT": MIT_TEXT, "COPYING.BSD": BSD_TEXT,
            "NOTICE.txt": "Fixture attribution notice.\n", "COPYRIGHT": "Additional fixture copyright.\n",
        })
        report = self.report()
        package = report.packages[0]
        self.assertEqual(report.workspace_count, 1)
        self.assertEqual(report.text_count, 4)
        self.assertEqual(report.unresolved, ())
        self.assertEqual(package.spdx, "MIT OR BSD-3-Clause")
        self.assertEqual({text.path: text.text for text in package.texts}, {
            "LICENSE-MIT": MIT_TEXT, "COPYING.BSD": BSD_TEXT,
            "NOTICE.txt": "Fixture attribution notice.\n", "COPYRIGHT": "Additional fixture copyright.\n",
        })
        rendered = report.render().decode("utf-8")
        self.assertIn(MIT_TEXT, rendered)
        self.assertIn(BSD_TEXT, rendered)
        self.assertIn("Package: fixture\nVersion: 1.2.3", rendered)
        self.assertIn("License (SPDX, as declared): MIT OR BSD-3-Clause", rendered)
        self.assertIn("not a legal-completeness determination", rendered)
        self.assertIn("No license is chosen or granted for Wren", rendered)
        self.assertNotIn("Package: wren-iced", rendered)

    def test_declared_license_file_is_relative_or_absolute_inside_root_and_deduplicated(self):
        root, package = self.crate(files={"legal/custom.terms": MIT_TEXT, "LICENSE": BSD_TEXT})
        for value in ("legal/custom.terms", str(root / "legal/custom.terms"), "LICENSE"):
            with self.subTest(value=value):
                package["license_file"] = value
                report = self.report()
                self.assertEqual(report.text_count, 2)
                self.assertEqual(report.unresolved, ())
                self.assertNotIn(str(root), report.render().decode("utf-8"))

    def test_only_bounded_notice_subdirectories_are_scanned_unless_explicitly_declared(self):
        _, package = self.crate(files={
            "LICENSES/MIT.txt": MIT_TEXT, "licenses/extra/attribution.txt": "Local extra notice.\n",
            "src/LICENSE": "Not automatically scanned.\n", "vendor/NOTICE": "Not automatically scanned either.\n",
            ".git/LICENSE": "Do not scan repository metadata.\n", "README.md": "Not a license file.\n",
        })
        report = self.report()
        self.assertEqual([text.path for text in report.packages[0].texts],
                         ["LICENSES/MIT.txt", "licenses/extra/attribution.txt"])
        self.assertNotIn("Not automatically scanned", report.render().decode("utf-8"))
        package["license_file"] = "src/LICENSE"
        self.assertEqual(report.text_count + 1, self.report().text_count)

    def test_spdx_only_unknown_and_undeclared_packages_have_explicit_gaps_no_fallback_grant(self):
        self.crate("spdx-only")
        self.crate("unknown", license="LicenseRef-Unknown")
        self.crate("undeclared", license=None, files={"COPYRIGHT": "Fixture attribution only.\n"})
        report = self.report()
        self.assertEqual(report.text_count, 1)
        self.assertEqual(report.unresolved, ("spdx-only@1.2.3", "undeclared@1.2.3", "unknown@1.2.3"))
        text = report.render().decode("utf-8")
        self.assertIn("Packages with no collected text: 2", text)
        self.assertIn("Text coverage: MISSING", text)
        self.assertIn("An SPDX declaration alone does not supply the license text", text)
        self.assertIn("License (SPDX, as declared): NOT DECLARED", text)
        self.assertIn("no permissions inferred", text)
        self.assertNotIn("Permission is hereby granted", text)
        self.assertNotIn("Copyright (c) 2026 Fixture Authors", text)

    def test_all_metadata_packages_include_transitive_dev_build_and_other_target_scope(self):
        for name in ("direct", "transitive", "dev-only", "build-only", "other-target"):
            self.crate(name, files={"LICENSE": MIT_TEXT})
        metadata = self.metadata()
        metadata["resolve"] = {"nodes": [{"id": self.own["id"], "deps": []}]}
        report = notices.notices_from_metadata(metadata)
        self.assertEqual(len(report.packages), 5)
        self.assertIn("includes build/dev dependencies", report.render().decode("utf-8"))
        self.assertIn("target-specific packages", report.render().decode("utf-8"))

    def test_output_is_deterministic_across_package_order_and_source_locations(self):
        self.crate("zebra", files={"NOTICE": "Zebra notice.\n", "LICENSE": MIT_TEXT})
        self.crate("alpha", files={"COPYING": BSD_TEXT})
        original = self.report().render()
        metadata = copy.deepcopy(self.metadata())
        metadata["packages"].reverse()
        relocated = self.base / "different absolute cache"
        relocated.mkdir()
        for index, package in enumerate(metadata["packages"]):
            if package["id"] == self.own["id"]:
                continue
            old = Path(package["manifest_path"]).parent
            new = relocated / f"source-{index}"
            shutil.copytree(old, new)
            package["manifest_path"] = str(new / "Cargo.toml")
            package["source"] = f"registry+https://different-private-source/{index}"
        self.assertEqual(original, notices.notices_from_metadata(metadata).render())

    def test_source_ids_urls_private_cache_and_metadata_provenance_are_not_emitted(self):
        root, package = self.crate(files={"LICENSE": MIT_TEXT})
        secret = "DO_NOT_EMIT_CREDENTIAL"
        url = f"registry+https://user:{secret}@private.example.invalid/index"
        package.update(id=url, source=url, repository=url, homepage=url, documentation=url,
                       authors=[secret], arbitrary_private_provenance=secret)
        text = self.report().render().decode("utf-8")
        for value in (secret, url, str(root), "private cache", "private-workspace-id", "private.example.invalid"):
            self.assertNotIn(value, text)
        self.assertIn(MIT_TEXT, text)

    def test_declared_parent_traversal_absolute_outside_and_windows_paths_are_rejected(self):
        root, package = self.crate()
        outside = self.base / "outside-secret.terms"
        outside.write_text("PRIVATE OUTSIDE FILE MUST NOT BE COPIED\n")
        for value in ("../../outside-secret.terms", "legal/../LICENSE", str(outside),
                      "C:\\private\\LICENSE", "C:/PRIVATE_OUTSIDE/LICENSE", "\x00LICENSE",
                      "https://user:PRIVATE_OUTSIDE@private.invalid/LICENSE", "file:/PRIVATE_OUTSIDE/LICENSE"):
            with self.subTest(value=value):
                package["license_file"] = value
                report = self.report()
                text = report.render().decode("utf-8")
                self.assertEqual(report.text_count, 0)
                self.assertEqual(report.unresolved, ("fixture@1.2.3",))
                self.assertIn("declared license_file rejected", text)
                self.assertNotIn("PRIVATE OUTSIDE FILE", text)
                self.assertNotIn("PRIVATE_OUTSIDE", text)
                self.assertNotIn("private.invalid", text)
                self.assertNotIn(str(outside), text)
                self.assertNotIn(str(root), text)

    def test_symlink_notice_files_and_license_directories_are_never_followed(self):
        root, package = self.crate()
        external = self.base / "private outside directory"
        external.mkdir()
        (external / "MIT.txt").write_text("SECRET OUTSIDE TEXT\n")
        (root / "LICENSE").symlink_to(external / "MIT.txt")
        (root / "licenses").symlink_to(external)
        (root / "custom").symlink_to(external)
        package["license_file"] = "custom/MIT.txt"
        report = self.report()
        self.assertEqual(report.text_count, 0)
        text = report.render().decode("utf-8")
        self.assertIn("symlinks are not followed", text)
        self.assertNotIn("SECRET OUTSIDE TEXT", text)
        self.assertNotIn(str(external), text)

    def test_package_root_and_ancestor_symlinks_are_unavailable_not_followed(self):
        root, package = self.crate(files={"LICENSE": "PRIVATE REAL ROOT TEXT\n"})
        link = self.base / "root-link"
        link.symlink_to(root)
        ancestor = self.base / "ancestor-link"
        ancestor.symlink_to(root.parent)
        for manifest in (link / "Cargo.toml", ancestor / root.name / "Cargo.toml"):
            with self.subTest(manifest=manifest):
                package["manifest_path"] = str(manifest)
                report = self.report()
                self.assertEqual(report.text_count, 0)
                self.assertIn("root/manifest unavailable or unsafe", report.render().decode("utf-8"))
                self.assertNotIn("PRIVATE REAL ROOT TEXT", report.render().decode("utf-8"))

    def test_missing_declared_file_and_missing_root_are_explicit_not_silently_complete(self):
        root, package = self.crate(license_file="missing.txt")
        text = self.report().render().decode("utf-8")
        self.assertIn('"missing.txt": file missing, unreadable, or unsafe', text)
        shutil.rmtree(root)
        report = self.report()
        self.assertEqual(report.unresolved, ("fixture@1.2.3",))
        self.assertIn("root/manifest unavailable or unsafe", report.render().decode("utf-8"))

    def test_undecodable_binary_empty_and_oversized_texts_are_explicitly_reported(self):
        self.crate(files={"LICENSE-invalid": b"\xff\xfe", "COPYING-binary": b"grant\x00end",
                          "NOTICE-empty": " \n\t", "COPYRIGHT-large": "x" * 33})
        with patch.object(notices, "MAX_TEXT_BYTES", 32):
            report = self.report()
        self.assertEqual(report.text_count, 0)
        text = report.render().decode("utf-8")
        for issue in ("not UTF-8 text", "binary/control characters", "empty text", "file size limit exceeded"):
            self.assertIn(issue, text)
        self.assertEqual(len(report.packages[0].issues), 4)

    def test_special_files_are_rejected_without_blocking(self):
        root, package = self.crate()
        os.mkfifo(root / "LICENSE")
        (root / "declared-directory").mkdir()
        package["license_file"] = "declared-directory"
        report = self.report()
        self.assertEqual(report.text_count, 0)
        self.assertEqual(sum("not a regular file" in issue for issue in report.packages[0].issues), 2)

    def test_manifest_symlink_is_not_treated_as_a_regular_package_manifest(self):
        root, _ = self.crate(files={"LICENSE": MIT_TEXT})
        manifest = root / "Cargo.toml"
        external = self.base / "external-manifest"
        manifest.rename(external)
        manifest.symlink_to(external)
        report = self.report()
        self.assertEqual(report.text_count, 0)
        self.assertEqual(report.unresolved, ("fixture@1.2.3",))

    def test_file_count_package_size_directory_entry_and_depth_limits_are_reported(self):
        root, _ = self.crate(files={"LICENSE-a": "alpha\n", "LICENSE-b": "beta\n"})
        with patch.object(notices, "MAX_FILES", 1):
            report = self.report()
            self.assertEqual(report.text_count, 0)
            self.assertIn("notice file count limit exceeded", report.render().decode("utf-8"))
        with patch.object(notices, "MAX_PACKAGE_BYTES", 8):
            report = self.report()
            self.assertEqual([text.path for text in report.packages[0].texts], ["LICENSE-a"])
            self.assertIn("package text size limit exceeded", report.render().decode("utf-8"))
        with patch.object(notices, "MAX_DIRECTORY_ENTRIES", 2):
            report = self.report()
            self.assertEqual(report.text_count, 0)
            self.assertIn("directory entry limit exceeded", report.render().decode("utf-8"))
        deep = root / "licenses/one/two"
        deep.mkdir(parents=True)
        (deep / "MIT").write_text("Not scanned beyond the bounded depth\n")
        report = self.report()
        self.assertEqual(report.text_count, 2)
        self.assertIn("notice directory depth limit exceeded", report.render().decode("utf-8"))
        self.assertNotIn("Not scanned beyond", report.render().decode("utf-8"))

    def test_relative_file_labels_escape_control_characters_and_never_expose_root(self):
        self.crate(files={"LICENSE-\nspoof.txt": MIT_TEXT})
        text = self.report().render().decode("utf-8")
        self.assertIn('--- Local file: "LICENSE-\\nspoof.txt" ---', text)
        self.assertNotIn(str(self.base), text)

    def test_invalid_spdx_is_not_used_to_inject_urls_or_grants(self):
        self.crate(license="https://user:SECRET@example.invalid/license", files={"LICENSE": MIT_TEXT})
        report = self.report()
        text = report.render().decode("utf-8")
        self.assertIn("invalid SPDX declaration omitted", text)
        self.assertNotIn("SECRET", text)
        self.assertEqual(report.unresolved, ("fixture@1.2.3",))

    def test_workspace_identity_must_be_present_and_consistent(self):
        self.crate()
        metadata = self.metadata()
        cases = [{"packages": metadata["packages"]},
                 {**metadata, "workspace_members": ["missing-member-private-secret"]},
                 {**metadata, "workspace_members": [self.own["id"], self.own["id"]]},
                 {**metadata, "packages": [*metadata["packages"], self.packages[0]]}]
        for malformed in cases:
            with self.subTest(malformed=malformed):
                with self.assertRaises(notices.NoticeError) as error:
                    notices.notices_from_metadata(malformed)
                self.assertNotIn("private-secret", str(error.exception))

    def test_malformed_package_name_version_and_manifest_do_not_leak_raw_values(self):
        _, package = self.crate()
        original = dict(package)
        for key, value in (("name", "https://SECRET@private.invalid"), ("version", "1.0\nSECRET"),
                           ("manifest_path", "/SECRET/../outside/Cargo.toml"),
                           ("manifest_path", "/SECRET/not-a-manifest")):
            with self.subTest(key=key, value=value):
                package.update(original)
                package[key] = value
                with self.assertRaises(notices.NoticeError) as error:
                    self.report()
                self.assertNotIn("SECRET", str(error.exception))

    def test_metadata_command_is_locked_offline_and_collection_can_be_injected(self):
        self.crate(files={"LICENSE": MIT_TEXT})
        completed = subprocess.CompletedProcess([], 0, json.dumps(self.metadata()).encode(), b"private stderr")
        with patch.object(notices.subprocess, "run", return_value=completed) as run:
            report = notices.collect_notices(self.base)
        run.assert_called_once_with(["cargo", "metadata", "--locked", "--format-version", "1", "--offline"],
                                    cwd=self.base, capture_output=True, check=False, timeout=120)
        self.assertEqual(report.text_count, 1)
        self.assertNotIn("private stderr", report.render().decode("utf-8"))

    def test_metadata_failure_invalid_json_and_spawn_errors_never_forward_cargo_output(self):
        secret = "https://user:SECRET@private.invalid/registry"
        for result in (subprocess.CompletedProcess([], 101, secret.encode(), secret.encode()),
                       subprocess.CompletedProcess([], 0, secret.encode(), secret.encode()),
                       subprocess.CompletedProcess([], 0, b"[]", secret.encode())):
            with self.subTest(result=result):
                with patch.object(notices.subprocess, "run", return_value=result):
                    with self.assertRaises(notices.NoticeError) as error:
                        notices.load_metadata(self.base)
                self.assertNotIn("SECRET", str(error.exception))
                self.assertNotIn("private.invalid", str(error.exception))
        for exception in (OSError(secret), subprocess.TimeoutExpired(secret, 120, output=secret, stderr=secret)):
            with patch.object(notices.subprocess, "run", side_effect=exception):
                with self.assertRaises(notices.NoticeError) as error:
                    notices.load_metadata(self.base)
                self.assertNotIn("SECRET", str(error.exception))

    def test_collector_cli_writes_report_and_summary_not_full_license_blob(self):
        self.crate(files={"LICENSE": MIT_TEXT})
        output = self.base / "DEPENDENCY_NOTICES.txt"
        report = self.report()
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(notices, "collect_notices", return_value=report) as collect:
            with redirect_stdout(stdout), redirect_stderr(stderr):
                self.assertEqual(notices.main(["--root", str(self.base), "--output", str(output)]), 0)
        collect.assert_called_once_with(self.base)
        self.assertEqual(output.read_bytes(), report.render())
        self.assertIn("1 packages, 1 local texts, 0 packages requiring review", stdout.getvalue())
        self.assertNotIn(MIT_TEXT, stdout.getvalue())
        self.assertEqual(stderr.getvalue(), "")
        with patch.object(notices, "collect_notices", return_value=report), redirect_stderr(stderr):
            self.assertEqual(notices.main(["--output", str(output)]), 1)
        self.assertEqual(output.read_bytes(), report.render())

    def test_collector_cli_generation_failure_and_symlink_output_do_not_publish(self):
        output = self.base / "new-report"
        stderr = io.StringIO()
        with patch.object(notices, "collect_notices", side_effect=notices.NoticeError("PRIVATE")):
            with redirect_stderr(stderr):
                self.assertEqual(notices.main(["--output", str(output)]), 1)
        self.assertFalse(output.exists())
        self.assertNotIn("PRIVATE", stderr.getvalue())
        external = self.base / "existing-report"
        external.write_bytes(b"preserve")
        output.symlink_to(external)
        with patch.object(notices, "collect_notices", return_value=self.report()), redirect_stderr(stderr):
            self.assertEqual(notices.main(["--output", str(output)]), 1)
        self.assertEqual(external.read_bytes(), b"preserve")


if __name__ == "__main__":
    unittest.main()
