"""Comparison helpers for the Copier parity scripts.

compare.py trees <copier-dir> <rootstock-dir> [--copier-markers <tag>]
    Compares two trees entry by entry: kind (file, symlink, directory),
    the executable bit, symlink targets and file bytes. `.git`, Copier's
    answers file and the rootstock record are skipped. With
    --copier-markers, Copier's conflict labels (`before updating`,
    `after updating`) are rewritten to rootstock's (`project`,
    `template <tag>`) before the bytes are compared, and each file where
    that happened is printed as a `listed:` line, which is not a failure.

compare.py answers <copier-answers.yml> <.rootstock.toml> <commit>
    Compares the answers (Copier's `_` keys aside) and checks that the
    record points at <commit>.

Prints one line per difference; exits 1 if there is any. Copier's
unmerged index entries are not compared: `.git` is skipped.
"""

import os
import stat
import sys
import tomllib

SKIP = {".git", ".copier-answers.yml", ".rootstock.toml"}


def entries(root):
    found = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = sorted(d for d in dirnames if d not in SKIP)
        for name in dirnames + filenames:
            if name in SKIP:
                continue
            full = os.path.join(dirpath, name)
            rel = os.path.relpath(full, root)
            st = os.lstat(full)
            if stat.S_ISLNK(st.st_mode):
                found[rel] = ("link", os.readlink(full))
            elif stat.S_ISDIR(st.st_mode):
                found[rel] = ("dir", None)
            else:
                with open(full, "rb") as f:
                    data = f.read()
                found[rel] = ("file", (bool(st.st_mode & stat.S_IXUSR), data))
    return found


def relabel(data, tag):
    lines = data.split(b"\n")
    for i, line in enumerate(lines):
        if line.rstrip(b"\r") == b"<<<<<<< before updating":
            lines[i] = b"<<<<<<< project" + line[len(b"<<<<<<< before updating"):]
        elif line.rstrip(b"\r") == b">>>>>>> after updating":
            end = line[len(b">>>>>>> after updating"):]
            lines[i] = b">>>>>>> template " + tag.encode() + end
    return b"\n".join(lines)


def trees(copier_dir, rootstock_dir, tag):
    a, b = entries(copier_dir), entries(rootstock_dir)
    out = []
    for rel in sorted(set(a) | set(b)):
        if rel not in a:
            out.append(f"only in rootstock: {rel}")
        elif rel not in b:
            out.append(f"only in copier: {rel}")
        elif a[rel][0] != b[rel][0]:
            out.append(f"kind differs: {rel} ({a[rel][0]} vs {b[rel][0]})")
        elif a[rel][0] == "link" and a[rel][1] != b[rel][1]:
            out.append(f"link target differs: {rel}")
        elif a[rel][0] == "file":
            (ax, ad), (bx, bd) = a[rel][1], b[rel][1]
            if tag is not None:
                relabelled = relabel(ad, tag)
                if relabelled != ad:
                    # A listed difference, not a failure: docs/design.md sets the labels.
                    print(f"listed: conflict labels rewritten in {rel}")
                ad = relabelled
            if ax != bx:
                out.append(f"executable bit differs: {rel}")
            if ad != bd:
                out.append(f"content differs: {rel}")
    return out


def answers(answers_file, record_file, commit):
    import yaml

    with open(answers_file) as f:
        copier = yaml.safe_load(f)
    with open(record_file, "rb") as f:
        record = tomllib.load(f)
    out = []
    if record.get("commit") != commit:
        out.append(f"record commit {record.get('commit')} != {commit}")
    asked = {k: v for k, v in copier.items() if not k.startswith("_")}
    recorded = record.get("answers", {})
    for key in sorted(set(asked) | set(recorded)):
        if asked.get(key, "<absent>") != recorded.get(key, "<absent>"):
            out.append(
                f"answer {key}: copier={asked.get(key, '<absent>')!r}"
                f" rootstock={recorded.get(key, '<absent>')!r}"
            )
    return out


def main(argv):
    if len(argv) >= 3 and argv[0] == "trees":
        tag = None
        if len(argv) == 5 and argv[3] == "--copier-markers":
            tag = argv[4]
        elif len(argv) != 3:
            sys.exit(__doc__)
        out = trees(argv[1], argv[2], tag)
    elif len(argv) == 4 and argv[0] == "answers":
        out = answers(argv[1], argv[2], argv[3])
    else:
        sys.exit(__doc__)
    for line in out:
        print(line)
    return 1 if out else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
