#!/usr/bin/env python3
"""Regenerate src/emoji/emoji.tsv: one emoji per line, `emoji \t group \t name \t keywords`.

Emoji, names and groups come from Unicode's emoji-test.txt (fully-qualified, no skin-tone
variants, no "Component" group). Extra search keywords come from Omarchy's emoji picker
(emojis.json, MIT licence), matched by emoji.

    scripts/gen-emoji.py [emoji-test.txt] [emojis.json] > src/emoji/emoji.tsv

Defaults: the copy of emoji-test.txt in the cargo registry (unicode-width's tests) and
/usr/share/omarchy/shell/plugins/emojis/emojis.json.
"""
import glob, json, os, re, sys

SKIN = {0x1F3FB, 0x1F3FC, 0x1F3FD, 0x1F3FE, 0x1F3FF}

def default_test_file():
    hits = sorted(glob.glob(os.path.expanduser(
        "~/.cargo/registry/src/*/unicode-width-*/tests/emoji-test.txt")))
    if not hits:
        sys.exit("emoji-test.txt not found; pass its path")
    return hits[-1]

def main():
    test = sys.argv[1] if len(sys.argv) > 1 else default_test_file()
    omarchy = sys.argv[2] if len(sys.argv) > 2 else "/usr/share/omarchy/shell/plugins/emojis/emojis.json"
    keywords = {}
    if os.path.exists(omarchy):
        for row in json.load(open(omarchy, encoding="utf-8")):
            keywords[row["e"].replace("️", "")] = row["k"]
    group = None
    line_re = re.compile(r"^([0-9A-F ]+?)\s*;\s*fully-qualified\s*#\s*(\S+)\s+E\d+\.\d+\s+(.+)$")
    for line in open(test, encoding="utf-8"):
        line = line.rstrip("\n")
        if line.startswith("# group:"):
            group = line.split(":", 1)[1].strip()
            continue
        m = line_re.match(line)
        if not m or group == "Component":
            continue
        cps = [int(c, 16) for c in m.group(1).split()]
        if SKIN & set(cps):
            continue
        emoji, name = m.group(2), m.group(3).strip()
        name_words = set(re.findall(r"[\w']+", name.lower()))
        extra = keywords.get(emoji.replace("️", ""), "")
        kw, seen = [], set()
        for w in re.findall(r"[\w'+-]+", extra.lower()):
            if w not in name_words and w not in seen:
                seen.add(w)
                kw.append(w)
        print(f"{emoji}\t{group}\t{name}\t{' '.join(kw)}")

main()
