#!/usr/bin/env python3
"""The one mechanical migration of `explicit-library` (D10): adds the library imports every
fixture, edit version and example needs once the prelude is gone, driven by the compiler's own
import help, then checks that the re-blessed snapshots differ only by shifted line numbers.

Usage (from the repository root, after `cargo build -p prism-cli`):

    migrate_imports.py [--dry-run] [--only <substring>]   insert the imports
    migrate_imports.py --verify-snapshots                 compare snapshots with HEAD

Insertion. For each fixture case (its header file, or `// entry:`, checked from the case
directory), each example file and each edit version (`*.prism.v<k>`, checked in a temporary
copy of its case with the version in place of its base file), `prism check --no-cache` runs and
every help line naming `add 'import /prism/...'` is collected with the file of the diagnostic
it follows. Each named import is inserted into that file: after its package declaration and
imports, at its sorted place among the imports, else after the leading header directives
(`// expect: ...`), with one blank line after the block when code follows. Rounds repeat until
no help is new (one missing import can hide another's errors), at most eight times.

Then the codes `prism check` reports must be exactly the case's expectation (`// expect:` or
`// expect-current:`). A case whose codes differ, which happens when a case tests a missing or
clashing name on purpose or when an inserted import clashes with the file's own declaration
(`E4010`), is restored and listed for a hand fix, as is every `// library:` case (its test
modules import what they use already) and every file the script could not settle.

Verification. Every snapshot (`*.snap`) that differs from HEAD under the fixtures and examples
is compared line by line with its HEAD version. A changed line is accepted only when it equals
the old line with each source position (`file.prism:L:C`, `@L:C`, the gutter `L |` of a
snippet) moved by the lines inserted above that position in its file, computed from the source
files' own diffs against HEAD (insertions only). Anything else is listed.
"""

import argparse
import concurrent.futures
import difflib
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# Temporary copies go beside the scratch files of the run, not in the system's directory.
if os.environ.get("MIGRATE_TMPDIR"):
    tempfile.tempdir = os.environ["MIGRATE_TMPDIR"]

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = ROOT / "crates/prism-cli/tests/fixtures"
EXAMPLES = ROOT / "examples"
PRISM = ROOT / "target/debug/prism"

DIRECTIVE = re.compile(
    r"^//\s*(expect|expect-current|FIXME\(decision\)|entry|run|events|headless|watch|library):"
)
HELP = re.compile(r"add 'import (/prism/[A-Za-z0-9_/]+)'")
LOCATION = re.compile(r"^\s*--> (.+?):(\d+):(\d+)\s*$")
CODE = re.compile(r"error\[(E\d{4})\]")
IMPORT_LINE = re.compile(r"^import\s+(\S+)\s*$")
PACKAGE_LINE = re.compile(r"^package\s+\S+")
VERSION = re.compile(r"^(.*\.prism)\.v\d+$")
MAX_ROUNDS = 8


def header_of(case: Path):
    """The case's header file and its directives, or None."""
    for path in sorted(case.rglob("*.prism")):
        text = path.read_text()
        if re.search(r"^// expect(-current)?:", text, re.M):
            directives = {}
            for line in text.splitlines():
                m = re.match(r"^//\s*([a-z-]+):\s*(.*)$", line)
                if m and m.group(1) not in directives:
                    directives[m.group(1)] = m.group(2).strip()
            return path, directives
    return None


def expected_codes(directives):
    value = directives.get("expect", directives.get("expect-current", "ok"))
    if value == "ok":
        return set()
    return set(re.split(r"[\s,]+", value.strip())) - {""}


def check(entry: Path, cwd: Path):
    """Runs `prism check` on `entry` from `cwd`: its help imports by file and its codes."""
    result = subprocess.run(
        [str(PRISM), "check", "--no-cache", os.path.relpath(entry, cwd)],
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=120,
    )
    helps = {}
    current = None
    codes = set()
    for line in (result.stdout + result.stderr).splitlines():
        codes.update(CODE.findall(line))
        m = LOCATION.match(line)
        if m:
            current = (cwd / m.group(1)).resolve()
            continue
        for module in HELP.findall(line):
            if current is not None:
                helps.setdefault(current, set()).add(module)
    return helps, codes


def import_key(path: str) -> str:
    """The order of D9: by path text, a root-anchored path with its leading `/`."""
    return path


def insert_imports(text: str, modules) -> str:
    """`text` with an import of each of `modules` it lacks, inserted lines only."""
    lines = text.split("\n")
    existing = {m.group(1) for m in map(IMPORT_LINE.match, lines) if m}
    new = sorted(set(modules) - existing, key=import_key)
    if not new:
        return text
    import_indices = [i for i, line in enumerate(lines) if IMPORT_LINE.match(line)]
    package_indices = [i for i, line in enumerate(lines) if PACKAGE_LINE.match(line)]
    if import_indices:
        block = [lines[i] for i in import_indices]
        contiguous = import_indices == list(range(import_indices[0], import_indices[-1] + 1))
        sorted_block = block == sorted(block, key=lambda l: import_key(l.split()[1]))
        if contiguous and sorted_block:
            # Each new import at its sorted place: a stable merge, inserting lines only.
            out = lines[: import_indices[0]]
            pending = list(new)
            for line in block:
                path = line.split()[1]
                while pending and import_key(pending[0]) < import_key(path):
                    out.append("import " + pending.pop(0))
                out.append(line)
            out.extend("import " + p for p in pending)
            out.extend(lines[import_indices[-1] + 1 :])
            return "\n".join(out)
        after = import_indices[-1]
        inserted = ["import " + p for p in new]
    elif package_indices:
        after = package_indices[0]
        inserted = [""] + ["import " + p for p in new]
    else:
        after = -1
        for i, line in enumerate(lines):
            if DIRECTIVE.match(line):
                after = i
            elif line.strip() == "":
                continue
            else:
                break
        inserted = ["import " + p for p in new]
    following = lines[after + 1] if after + 1 < len(lines) else ""
    if following.strip() != "":
        inserted.append("")
    return "\n".join(lines[: after + 1] + inserted + lines[after + 1 :])


def settle(entry: Path, cwd: Path, files_root: Path, rename=None):
    """Runs the fixpoint for `entry`: returns the final codes and the modules inserted by file.

    `rename` maps a checked path to the file to edit instead (an edit version checked in place
    of its base file)."""
    inserted = {}
    codes = set()
    for _ in range(MAX_ROUNDS):
        helps, codes = check(entry, cwd)
        progress = False
        for path, modules in helps.items():
            if not str(path).startswith(str(files_root.resolve())):
                continue
            target = rename(path) if rename else path
            if target is None:
                continue
            text = target.read_text()
            updated = insert_imports(text, modules)
            if updated != text:
                target.write_text(updated)
                if rename:
                    # Keep the checked copy in step with the version.
                    path.write_text(updated)
                inserted.setdefault(target, set()).update(modules)
                progress = True
        if not progress:
            break
    return codes, inserted


def migrate_case(case: Path, dry_run: bool):
    """Migrates one fixture case; returns (case, status, details)."""
    case = case.resolve()
    header = header_of(case)
    if header is None:
        return case, "no-header", ""
    header_file, directives = header
    if "library" in directives:
        return case, "library", ""
    entry = case / directives["entry"] if "entry" in directives else header_file
    expected = expected_codes(directives)
    originals = {p: p.read_text() for p in case.rglob("*") if p.is_file()}
    work = case
    if dry_run:
        tmp = Path(tempfile.mkdtemp(prefix="migrate-"))
        work = (tmp / case.name).resolve()
        shutil.copytree(case, work)
        entry = work / entry.relative_to(case)
    try:
        codes, inserted = settle(entry, work, work)
        details = []
        if codes != expected:
            if not dry_run:
                for path, text in originals.items():
                    path.write_text(text)
            return case, "mismatch", f"codes {sorted(codes)} != expected {sorted(expected)}; would insert {fmt(inserted, work)}"
        for path, modules in inserted.items():
            details.append(f"{path.relative_to(work)}: {', '.join(sorted(modules))}")
        # Edit versions, each checked in a copy of the (migrated) case.
        for version in sorted(work.rglob("*.prism.v*")):
            m = VERSION.match(version.name)
            if not m:
                continue
            base = version.with_name(m.group(1))
            vtmp = Path(tempfile.mkdtemp(prefix="migrate-version-"))
            try:
                copy = (vtmp / case.name).resolve()
                shutil.copytree(work, copy)
                copy_base = copy / base.relative_to(work)
                copy_base.write_text(version.read_text())
                copy_entry = copy / entry.relative_to(work)
                resolved_base = copy_base.resolve()

                def rename(path, resolved_base=resolved_base, version=version):
                    return version if path == resolved_base else None

                _, vinserted = settle(copy_entry, copy, copy, rename)
                # `rename` points into the case, outside the copy: apply directly.
                for path, modules in vinserted.items():
                    details.append(f"{path.relative_to(work)}: {', '.join(sorted(modules))}")
            finally:
                shutil.rmtree(vtmp, ignore_errors=True)
        return case, "ok" if details else "unchanged", "; ".join(details)
    finally:
        if dry_run:
            shutil.rmtree(work.parent, ignore_errors=True)


def fmt(inserted, root):
    return "; ".join(
        f"{p.relative_to(root) if str(p).startswith(str(root)) else p}: {', '.join(sorted(m))}"
        for p, m in inserted.items()
    )


def migrate_examples(dry_run: bool):
    results = []
    entries = sorted(EXAMPLES.rglob("*.prism"))
    for entry in entries:
        codes, inserted = settle(entry, ROOT, EXAMPLES) if not dry_run else (set(), {})
        if dry_run:
            helps, codes = check(entry, ROOT)
            inserted = helps
        results.append((entry, codes, inserted))
    # Session versions: `<session>.<file>.v<k>` replacing `../<file>`.
    for version in sorted(EXAMPLES.rglob("sessions/*.v*")):
        m = re.match(r"^[^.]+\.(.+\.prism)\.v\d+$", version.name)
        if not m:
            continue
        base = version.parent.parent / m.group(1)
        original = base.read_text()
        try:
            base.write_text(version.read_text())
            app_entry = base.parent / "main.prism"
            if dry_run:
                helps, codes = check(app_entry, ROOT)
                results.append((version, codes, helps))
            else:
                codes, inserted = settle_version(app_entry, base, version)
                results.append((version, codes, inserted))
        finally:
            base.write_text(original)
    return results


def settle_version(entry: Path, base: Path, version: Path):
    inserted = set()
    codes = set()
    for _ in range(MAX_ROUNDS):
        helps, codes = check(entry, ROOT)
        modules = helps.get(base.resolve(), set())
        text = version.read_text()
        updated = insert_imports(text, modules)
        if updated == text:
            break
        version.write_text(updated)
        base.write_text(updated)
        inserted |= modules
    return codes, {version: inserted} if inserted else {}


# ----------------------------------------------------------------------------------------------
# Snapshot verification


def git_show(path: Path):
    rel = path.relative_to(ROOT)
    result = subprocess.run(
        ["git", "show", f"HEAD:{rel}"], cwd=ROOT, capture_output=True, text=True
    )
    return result.stdout if result.returncode == 0 else None


def line_mapping(old: str, new: str):
    """Old line number -> new line number, or None when the diff is not insertions only."""
    a, b = old.split("\n"), new.split("\n")
    mapping = {}
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes():
        if tag == "equal":
            for k in range(i2 - i1):
                mapping[i1 + k + 1] = j1 + k + 1
        elif tag != "insert":
            return None
    return mapping


POSITION = re.compile(r"((?:[\w./-]+\.prism(?:\.v\d+)?):(\d+):(\d+))|(@(\d+):(\d+))")
GUTTER = re.compile(r"^(\s*)(\d+)(\s+\|.*)$")
BLANK_GUTTER = re.compile(r"^\s*\|(.*)$")


def case_of(snapshot: Path):
    """The directory holding the sources a snapshot's positions refer to."""
    if snapshot.parent.name == "sessions":
        return snapshot.parent.parent
    return snapshot.parent


def mappings_for(case: Path):
    maps = {}
    identity = True
    for path in case.rglob("*"):
        if not path.is_file() or not (path.suffix == ".prism" or VERSION.match(path.name)):
            continue
        old = git_show(path)
        if old is None:
            continue
        new = path.read_text()
        mapping = line_mapping(old, new)
        maps[path.resolve()] = mapping
    return maps


def verify_snapshots():
    changed = subprocess.run(
        ["git", "diff", "--name-only", "HEAD", "--", "crates/prism-cli/tests/fixtures", "examples"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    ).stdout.split()
    snapshots = [ROOT / p for p in changed if p.endswith(".snap")]
    problems = []
    accepted = 0
    for snapshot in snapshots:
        old = git_show(snapshot)
        if old is None or not snapshot.exists():
            problems.append(f"{snapshot.relative_to(ROOT)}: added or removed")
            continue
        new = snapshot.read_text()
        case = case_of(snapshot)
        maps = mappings_for(case)
        bad_maps = [p for p, m in maps.items() if m is None]
        if bad_maps:
            problems.append(
                f"{snapshot.relative_to(ROOT)}: sources changed beyond insertions: "
                + ", ".join(str(p.relative_to(ROOT)) for p in bad_maps)
            )
            continue
        issues = compare(old, new, maps)
        if issues:
            problems.append(f"{snapshot.relative_to(ROOT)}:\n    " + "\n    ".join(issues[:6]))
        else:
            accepted += 1
    print(f"{accepted} changed snapshots differ only by shifted line numbers")
    if problems:
        print(f"{len(problems)} to read by hand:")
        for problem in problems:
            print("  " + problem)
        return 1
    return 0


def compare(old: str, new: str, maps):
    """The lines of `new` that are not `old`'s with shifted positions."""
    a, b = old.split("\n"), new.split("\n")
    if len(a) != len(b):
        return [f"line count {len(a)} -> {len(b)}"]
    issues = []
    by_name = {}
    for path, mapping in maps.items():
        # A diagnostic in an edit version names its base file (`main.prism` for
        # `main.prism.v2`), so a position of the base may be one of any of its versions.
        version = VERSION.match(path.name)
        by_name.setdefault(version.group(1) if version else path.name, []).append(mapping)
    all_maps = list(maps.values())
    current = None  # the mappings of the file of the latest `-->` line
    for old_line, new_line in zip(a, b):
        loc = LOCATION.match(old_line)
        if loc:
            name = Path(loc.group(1)).name
            current = by_name.get(name, all_maps)
        if old_line == new_line:
            continue
        if not shifted(old_line, new_line, by_name, all_maps, current):
            issues.append(f"- {old_line}\n    + {new_line}")
    return issues


def maps_line(mappings, old, new):
    return any(m is not None and m.get(old) == new for m in mappings)


def shifted(old_line, new_line, by_name, all_maps, current):
    # A snippet's gutter: `  12 |     code`.
    go, gn = GUTTER.match(old_line), GUTTER.match(new_line)
    if go and gn:
        return go.group(3).strip() == gn.group(3).strip() and maps_line(
            current or all_maps, int(go.group(2)), int(gn.group(2))
        )
    bo, bn = BLANK_GUTTER.match(old_line), BLANK_GUTTER.match(new_line)
    if bo and bn:
        # A wider gutter only moves the bar.
        return bo.group(1).rstrip() == bn.group(1).rstrip() or bo.group(1) == bn.group(1)
    po, pn = list(POSITION.finditer(old_line)), list(POSITION.finditer(new_line))
    if len(po) != len(pn) or not po:
        return False
    rebuilt_old, rebuilt_new = [], []
    last_o = last_n = 0
    for mo, mn in zip(po, pn):
        rebuilt_old.append(old_line[last_o : mo.start()])
        rebuilt_new.append(new_line[last_n : mn.start()])
        last_o, last_n = mo.end(), mn.end()
        if mo.group(1) and mn.group(1):
            name_o = mo.group(1).rsplit(":", 2)[0]
            name_n = mn.group(1).rsplit(":", 2)[0]
            if name_o != name_n or mo.group(3) != mn.group(3):
                return False
            mappings = by_name.get(Path(name_o).name, all_maps)
            if not maps_line(mappings, int(mo.group(2)), int(mn.group(2))):
                return False
        elif mo.group(4) and mn.group(4):
            if mo.group(6) != mn.group(6):
                return False
            if not maps_line(all_maps, int(mo.group(5)), int(mn.group(5))):
                return False
        else:
            return False
    rebuilt_old.append(old_line[last_o:])
    rebuilt_new.append(new_line[last_n:])
    return rebuilt_old == rebuilt_new


# ----------------------------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--verify-snapshots", action="store_true")
    parser.add_argument("--only", default=None)
    parser.add_argument("--no-examples", action="store_true")
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 4)
    args = parser.parse_args()
    if args.verify_snapshots:
        sys.exit(verify_snapshots())
    if not PRISM.exists():
        sys.exit(f"build the compiler first: {PRISM} is missing")
    cases = sorted(d for d in FIXTURES.glob("*/*") if d.is_dir())
    if args.only:
        cases = [c for c in cases if args.only in str(c)]
    listed = []
    changed = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for case, status, details in pool.map(lambda c: migrate_case(c, args.dry_run), cases):
            rel = case.relative_to(FIXTURES.resolve())
            if status == "ok":
                changed += 1
                if args.dry_run:
                    print(f"{rel}: {details}")
            elif status in ("mismatch", "library", "no-header"):
                listed.append(f"{rel}: {status} {details}")
    if not args.no_examples and not args.only:
        for path, codes, inserted in migrate_examples(args.dry_run):
            rel = path.relative_to(ROOT)
            shown = {Path(p).name: sorted(m) for p, m in inserted.items()}
            print(f"{rel}: {shown}" + (f" codes {sorted(codes)}" if codes else ""))
    print(f"{changed} cases gained imports")
    print(f"{len(listed)} cases to settle by hand:")
    for line in listed:
        print("  " + line)


if __name__ == "__main__":
    main()
