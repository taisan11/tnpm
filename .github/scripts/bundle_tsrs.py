"""Bundle the native tsrs executable and adjacent TypeScript library files.

The bundle is embedded into the released tnpm binary at build time; no
downloads or npm packages are needed at runtime.
"""
from __future__ import annotations

import os
from pathlib import Path
import subprocess
import tarfile
import tempfile

VERSION = "0.23.0-ts7.1.0-beta"


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="tnpm-tsrs-") as temporary:
        root = Path(temporary)
        subprocess.run(
            [
                "npm", "install", "--prefix", str(root),
                "--ignore-scripts", "--no-save", "--no-package-lock",
                "--no-audit", "--no-fund", f"tsrs@{VERSION}",
            ],
            check=True,
        )
        scope = root / "node_modules" / "@tsrs"
        native = [
            path for path in scope.iterdir()
            if path.is_dir() and (
                path / ("tsrs.exe" if os.name == "nt" else "tsrs")
            ).is_file()
        ]
        if len(native) != 1:
            raise RuntimeError(f"expected one native tsrs package, found {native!r}")

        source = native[0]
        if not (source / "lib.d.ts").is_file():
            raise RuntimeError("native tsrs package has no TypeScript default libraries")

        bundle = Path("target/tsrs-bundle.tar.gz").resolve()
        bundle.parent.mkdir(parents=True, exist_ok=True)
        with tarfile.open(bundle, "w:gz") as archive:
            for entry in source.rglob("*"):
                if entry.is_file():
                    archive.add(entry, arcname=entry.relative_to(source))
            # Keep upstream license and attribution alongside the embedded
            # compiler's files. These are not used by the compiler at runtime.
            for name in ("LICENSE", "NOTICE"):
                candidate = root / "node_modules" / "tsrs" / name
                if candidate.is_file():
                    archive.add(candidate, arcname=f"licenses/tsrs-{name}")

        print(f"Bundled tsrs {VERSION}: {source.name} -> {bundle}")
        github_env = os.environ.get("GITHUB_ENV")
        if github_env:
            with open(github_env, "a", encoding="utf-8") as output:
                output.write(f"TNPM_TSRS_BUNDLE={bundle}\n")


if __name__ == "__main__":
    main()
