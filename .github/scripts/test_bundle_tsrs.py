import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import bundle_tsrs


class NpmLauncherTests(unittest.TestCase):
    def test_missing_npm_reports_how_to_install_it(self):
        with patch.object(bundle_tsrs.shutil, "which", return_value=None):
            with self.assertRaisesRegex(RuntimeError, "npm was not found on PATH; install Node.js"):
                bundle_tsrs.main()

    def test_resolved_windows_launcher_is_used(self):
        npm = r"C:\Program Files\nodejs\npm.cmd"
        failure = subprocess.CalledProcessError(1, [npm])
        with patch.object(bundle_tsrs.shutil, "which", return_value=npm) as which:
            with patch.object(bundle_tsrs.subprocess, "run", side_effect=failure) as run:
                with self.assertRaises(subprocess.CalledProcessError):
                    bundle_tsrs.main()
        which.assert_called_once_with("npm")
        self.assertEqual(run.call_args.args[0][0], npm)
        self.assertTrue(run.call_args.kwargs["check"])

    def test_successful_install_bundles_native_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            native = root / "node_modules" / "@tsrs" / "native"
            native.mkdir(parents=True)
            (native / ("tsrs.exe" if bundle_tsrs.os.name == "nt" else "tsrs")).write_bytes(b"binary")
            (native / "lib.d.ts").write_text("declare const value: string;")
            with patch.object(bundle_tsrs.shutil, "which", return_value="/bin/npm"):
                with patch.object(bundle_tsrs.tempfile, "TemporaryDirectory") as temporary:
                    temporary.return_value.__enter__.return_value = directory
                    with patch.object(bundle_tsrs.subprocess, "run"):
                        with patch.object(bundle_tsrs.tarfile, "open") as archive:
                            with patch.dict(bundle_tsrs.os.environ, {}, clear=True):
                                bundle_tsrs.main()
            self.assertEqual(archive.return_value.__enter__.return_value.add.call_count, 2)


if __name__ == "__main__":
    unittest.main()
