#!/usr/bin/env python3
"""Adds the per-release Artifact Hub annotations to a chart's Chart.yaml before it is packaged.

    deploy/helm/artifacthub-annotate.py <chart dir> <version>

- artifacthub.io/images: every image the chart's default install runs (read from `helm template`), so
  Artifact Hub scans them for vulnerabilities;
- artifacthub.io/changes: this version's entries from CHANGELOG.md (written by release-please).

The release workflow runs it on a fresh checkout; the annotations are not committed. Standard library only.
"""

import pathlib
import re
import subprocess
import sys

KINDS = {"Features": "added", "Bug fixes": "fixed", "Security": "security", "Performance": "changed", "Documentation": "changed", "Dependencies": "changed"}


def images(chart: pathlib.Path) -> list[str]:
    ci_values = chart / "ci" / "default-values.yaml"
    args = ["helm", "template", "artifacthub", str(chart)] + (["-f", str(ci_values)] if ci_values.exists() else [])
    rendered = subprocess.run(args, check=True, capture_output=True, text=True).stdout
    found = re.findall(r'^\s*(?:-\s*)?image:\s*"?([^"\s]+)"?\s*$', rendered, re.M)
    return sorted(set(found))


def changes(changelog: pathlib.Path, version: str) -> list[tuple[str, str]]:
    section = re.search(rf"^## \[?{re.escape(version)}\]?.*?$(.*?)(?=^## |\Z)", changelog.read_text(), re.M | re.S)
    if not section:
        return []
    out, kind = [], "changed"
    for line in section.group(1).splitlines():
        if line.startswith("### "):
            kind = KINDS.get(line[4:].strip(), "changed")
        elif line.startswith("* "):
            # "* text ([#2](url)) ([6f7511a](url))" → "text"
            text = re.sub(r"\s*\(\[[^\]]+\]\([^)]+\)\)", "", line[2:]).strip()
            text = re.sub(r"^\*\*([^*]+):\*\*\s*", r"\1: ", text)
            out.append((kind, text))
    return out


def quote(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main() -> None:
    chart, version = pathlib.Path(sys.argv[1]), sys.argv[2]
    chart_yaml = chart / "Chart.yaml"
    text = chart_yaml.read_text()
    # The annotations are appended to the `annotations:` map, which must be the file's last key.
    top_level = [line for line in text.splitlines() if line and not line[0].isspace() and not line.startswith("#")]
    if not top_level or top_level[-1].rstrip() != "annotations:":
        sys.exit(f"{chart_yaml}: `annotations:` must be the last key")
    lines = ["  artifacthub.io/images: |"]
    for ref in images(chart):
        name = ref.split("@")[0].rsplit(":", 1)[0].rsplit("/", 1)[-1]
        lines += [f"    - name: {name}", f"      image: {ref}"]
    entries = changes(chart.parent.parent.parent / "CHANGELOG.md", version)
    if entries:
        lines.append("  artifacthub.io/changes: |")
        for kind, desc in entries:
            lines += [f"    - kind: {kind}", f"      description: {quote(desc)}"]
    chart_yaml.write_text(text.rstrip("\n") + "\n" + "\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
