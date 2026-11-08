#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""What the compiler can't see between the UI and the core, and what the Jolla Store refuses.

Commands  every command the UI or the bridge sends exists in the core (core/src/app.rs, `Command`),
          has every field it needs, and has no field the core doesn't know: serde ignores those,
          so a typo just loses the value. A command nothing sends is reported too.
Imports   QML imports only what the Jolla Store's validator allows, in both flavours: QtQuick 2.6
          at most (the list is the SDK's allowed_qmlimports.conf, shortened to what could be
          meant; the validator itself only sees the Lite RPM of a release pull request).
Tests     every program under tests/ belongs to a scripts/test-*.sh, which scripts/run-sdk-tests.sh
          then runs.
--texts   every text of qsTrId/qtTrId is in all translations/*.ts, with its //% comment, and
          translated: no unfinished entry in a language's file (scripts/translations-*.py leave a
          text unfinished whose English changed), plural forms filled in the English one. New
          texts get there only in the last step before a release (CLAUDE.md §6), so this is apart:
          for scripts/check.sh --full and for releases.

Usage: scripts/check-qml.py [--texts]
"""
import pathlib
import re
import sys
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent

ALLOWED_IMPORTS = {
    "Amber.Mpris": {"1.0"},
    "Nemo.Configuration": {"1.0"},
    "Nemo.DBus": {"2.0"},
    "Nemo.KeepAlive": {"1.2"},
    "Nemo.Notifications": {"1.0"},
    "Nemo.Thumbnailer": {"1.0"},
    "Nemo.Time": {"1.0"},
    "QtFeedback": {"5.0"},
    "QtGraphicalEffects": {"1.0"},
    "QtLocation": {"5.0", "5.3", "5.4"},
    "QtMultimedia": {"5.0", "5.1", "5.2", "5.3", "5.4", "5.5", "5.6"},
    "QtPositioning": {"5.2", "5.4"},
    "QtQml": {"2.0", "2.1", "2.2"},
    "QtQml.Models": {"2.1", "2.2", "2.3"},
    "QtQuick": {"2.0", "2.1", "2.2", "2.3", "2.4", "2.5", "2.6"},
    "QtQuick.Layouts": {"1.0", "1.1"},
    "QtQuick.Window": {"2.0", "2.1", "2.2"},
    "QtSensors": {"5.0", "5.1", "5.2"},
    "Sailfish.Pickers": {"1.0"},
    "Sailfish.Share": {"1.0"},
    "Sailfish.Silica": {"1.0"},
    "org.nemomobile.contacts": {"1.0"},
    # Our own, registered by the app itself.
    "harbour.threesea": {"1.0"},
    # The map renderer's module, shipped in the package (scripts/maplibre; the validator takes a
    # module the package brings along).
    "MapboxMap": {"1.0"},
}

# Commands that no UI sends, and why that is right.
NO_UI_CALLER = {
    "demoData": "emulator builds: asked for over D-Bus, behind CONFIG+=demo",
}


def camel(name):
    head, *rest = name.split("_")
    return head + "".join(part.capitalize() for part in rest)


def core_commands():
    """{command: {field: required}} from the enum."""
    source = (ROOT / "core/src/app.rs").read_text()
    start = source.index("pub enum Command {")
    body = source[start : source.index("\n}\n", start)]
    commands = {}
    current = None
    default = False
    for line in body.splitlines()[1:]:
        variant = re.match(r"    ([A-Z]\w*)\s*(\{|,)", line)
        field = re.match(r"        (\w+): (.*),$", line)
        if variant:
            current = camel(re.sub(r"(?<!^)([A-Z])", r"_\1", variant.group(1)).lower())
            commands[current] = {}
            default = False
        elif line.strip().startswith("#[serde(default"):
            default = True
        elif field and current:
            required = not default and not field.group(2).startswith("Option<")
            commands[current][camel(field.group(1))] = required
            default = False
    return commands


def strip_comment_aware(text):
    """The text with comments and the contents of strings blanked, offsets unchanged."""
    out = list(text)
    i = 0
    while i < len(text):
        c = text[i]
        if text.startswith("//", i):
            while i < len(text) and text[i] != "\n":
                out[i] = " "
                i += 1
        elif text.startswith("/*", i):
            end = text.find("*/", i + 2)
            end = len(text) if end < 0 else end + 2
            for j in range(i, end):
                if text[j] != "\n":
                    out[j] = " "
            i = end
        elif c in "\"'":
            j = i + 1
            while j < len(text) and text[j] != c:
                if text[j] == "\\":
                    out[j] = " "
                    j += 1
                if j < len(text):
                    if text[j] != "\n":
                        out[j] = " "
                j += 1
            i = j + 1
        else:
            i += 1
    return "".join(out)


def closing(blank, start):
    """The offset of what closes the bracket at `start`."""
    depth = 0
    for i in range(start, len(blank)):
        if blank[i] in "([{":
            depth += 1
        elif blank[i] in ")]}":
            depth -= 1
            if depth == 0:
                return i
    raise ValueError("unbalanced")


def top_level(blank, start, end):
    """The parts of blank[start:end] between its top-level commas, as (start, end)."""
    parts = []
    depth = 0
    begin = start
    for i in range(start, end):
        if blank[i] in "([{":
            depth += 1
        elif blank[i] in ")]}":
            depth -= 1
        elif blank[i] == "," and depth == 0:
            parts.append((begin, i))
            begin = i + 1
    if blank[begin:end].strip():
        parts.append((begin, end))
    return parts


def object_keys(text, blank, start):
    """The keys of the object literal at `start`."""
    end = closing(blank, start)
    keys = set()
    for begin, stop in top_level(blank, start + 1, end):
        key = re.match(r"\s*(?:(\w+)|([\"'])(\w+)\2)\s*:", text[begin:stop])
        if key:
            keys.add(key.group(1) or key.group(3))
    return keys


def local_object(text, blank, position, name):
    """The keys of `var name = {...}` nearest before `position`, with later `name.key = `, or None."""
    found = None
    for match in re.finditer(r"\bvar\s+%s\s*=\s*\{" % re.escape(name), blank[:position]):
        found = match
    if not found:
        return None
    start = found.end() - 1
    keys = object_keys(text, blank, start)
    for match in re.finditer(r"\b%s\.(\w+)\s*=[^=]" % re.escape(name), blank[start:position]):
        keys.add(match.group(1))
    return keys


def check_commands(errors):
    commands = core_commands()
    called = set()
    unchecked = 0
    sources = sorted(ROOT.glob("qml/**/*.qml")) + sorted(ROOT.glob("qml/**/*.js"))
    for path in sources:
        text = path.read_text()
        blank = strip_comment_aware(text)
        for match in re.finditer(r"\bbackend\.call\s*\(", blank):
            where = "%s:%d" % (path.relative_to(ROOT), text.count("\n", 0, match.start()) + 1)
            open_at = match.end() - 1
            parts = top_level(blank, open_at + 1, closing(blank, open_at))
            name = re.fullmatch(r"\s*\"(\w+)\"\s*", text[parts[0][0] : parts[0][1]])
            if not name:
                unchecked += 1
                continue
            name = name.group(1)
            called.add(name)
            if name not in commands:
                errors.append("%s: the core has no command `%s`" % (where, name))
                continue
            fields = commands[name]
            keys = set()
            if len(parts) > 1:
                argument = text[parts[1][0] : parts[1][1]].strip()
                start = parts[1][0] + len(text[parts[1][0] : parts[1][1]]) - len(
                    text[parts[1][0] : parts[1][1]].lstrip()
                )
                if argument.startswith("{"):
                    keys = object_keys(text, blank, start)
                else:
                    keys = local_object(text, blank, match.start(), argument)
                    if keys is None:
                        unchecked += 1
                        continue
            for key in sorted(keys - fields.keys()):
                errors.append("%s: `%s` has no field `%s`" % (where, name, key))
            for key in sorted(k for k, required in fields.items() if required and k not in keys):
                errors.append("%s: `%s` needs `%s`" % (where, name, key))
    for path in sorted(ROOT.glob("bridge/*.cpp")):
        text = path.read_text()
        for pattern in (
            r"\bcall\(QStringLiteral\(\"(\w+)\"\)",
            r"QStringLiteral\(\"cmd\"\),\s*QStringLiteral\(\"(\w+)\"\)",
            r"QStringLiteral\(\"cmd\"\),\s*QLatin1String\(\"(\w+)\"\)",
        ):
            for match in re.finditer(pattern, text):
                called.add(match.group(1))
                if match.group(1) not in commands:
                    line = text.count("\n", 0, match.start()) + 1
                    errors.append("bridge/%s:%d: the core has no command `%s`" % (path.name, line, match.group(1)))
    # Pages also pass a command's name on to a helper of theirs (`page.run("leaveGroup", ...)`),
    # and the call managers to theirs: there it is only seen as a quoted name.
    quoted = set()
    for path in sources + sorted(ROOT.glob("bridge/*.cpp")):
        quoted.update(re.findall(r"\"(\w+)\"", path.read_text()))
    for name in sorted(commands.keys() - called - quoted - NO_UI_CALLER.keys()):
        errors.append("core/src/app.rs: nothing sends the command `%s`" % name)
    return len(called), unchecked


def check_imports(errors):
    for path in sorted(ROOT.glob("qml/**/*.qml")):
        for number, line in enumerate(path.read_text().splitlines(), 1):
            match = re.match(r"\s*import\s+([A-Za-z][\w.]*)\s+(\d+\.\d+)", line)
            if not match:
                continue
            module, version = match.groups()
            if version not in ALLOWED_IMPORTS.get(module, ()):
                errors.append(
                    "%s:%d: import %s %s is not allowed" % (path.relative_to(ROOT), number, module, version)
                )


def check_tests(errors):
    scripts = "\n".join(path.read_text() for path in (ROOT / "scripts").glob("test-*.sh"))
    for path in sorted((ROOT / "tests").iterdir()):
        if path.is_dir() and "tests/%s" % path.name not in scripts:
            errors.append("tests/%s: no scripts/test-*.sh builds it" % path.name)


def text_ids():
    """{id: whether a //% comment stands right before one of its uses}"""
    ids = {}
    sources = sorted(ROOT.glob("qml/**/*.qml")) + sorted(ROOT.glob("qml/**/*.js")) + sorted(ROOT.glob("bridge/*.cpp"))
    for path in sources:
        lines = path.read_text().splitlines()
        for number, line in enumerate(lines):
            for text_id in re.findall(r"\b(?:qsTrId|qtTrId|QT_TRID_NOOP)\(\s*\"([^\"]+)\"", line):
                before = lines[max(0, number - 8) : number + 1]
                ids[text_id] = ids.get(text_id, False) or any("//%" in other for other in before)
    return ids


def check_texts(errors):
    ids = text_ids()
    for path in sorted((ROOT / "translations").glob("*.ts")):
        known = set(re.findall(r'<message id="([^"]+)"', path.read_text()))
        for text_id in sorted(ids.keys() - known):
            errors.append("%s: no %s" % (path.relative_to(ROOT), text_id))
    for text_id, commented in sorted(ids.items()):
        if not commented:
            errors.append("no //% comment for " + text_id)
    for path in sorted((ROOT / "translations").glob("*.ts")):
        base = path.name == "harbour-threesea.ts"
        for message in ET.parse(path).iter("message"):
            translation = message.find("translation")
            if message.get("numerus") == "yes":
                done = all((form.text or "").strip() for form in translation.iter("numerusform"))
            else:
                done = base or (translation.get("type") != "unfinished" and (translation.text or "").strip())
            if not done:
                errors.append("%s: not translated: %s" % (path.relative_to(ROOT), message.get("id")))


def main():
    args = sys.argv[1:]
    if args not in ([], ["--texts"]):
        sys.exit(__doc__)
    errors = []
    if args:
        check_texts(errors)
    else:
        check_commands(errors)
        check_imports(errors)
        check_tests(errors)
    for error in errors:
        print(error)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
