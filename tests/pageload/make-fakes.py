#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Writes the fake C++ side for tests/pageload from the bridge's own headers.

A fake written by hand can have what the real class lacks, and then a page calling that passes
here and fails on the phone. So each fake object has the real class's properties (read-only where
the class has no setter), invokables, slots and signals, nothing more. What the test needs beyond
empty values is in the tables below: values, and the few things it has to change while running.

A read of something a real object lacks is just undefined in QML, with no warning at all, so
--check looks for that in the sources.

Usage: make-fakes.py <bridge dir> <full|lite> <output dir>
       make-fakes.py --check <bridge dir> <full|lite> <qml dir>
"""
import os
import re
import sys

# Context property -> (header, class).
OBJECTS = {
    "backend": ("threesebackend.h", "ThreeseBackend"),
    "dbus": ("dbusinterface.h", "DBusInterface"),
    "lifecycle": ("applifecycle.h", "AppLifecycle"),
    "calls": ("callmanager.h", "CallManager"),
    "groupCalls": ("groupcallmanager.h", "GroupCallManager"),
    "voicePlayer": ("voiceplayer.h", "VoicePlayer"),
    "videoPlayer": ("videoplayer.h", "VideoPlayer"),
    "voiceRecorder": ("voicerecorder.h", "VoiceRecorder"),
    "proximity": ("proximitymonitor.h", "ProximityMonitor"),
    "softSound": ("softsound.h", "SoftSound"),
}
# C++ items as plain QML items with the class's properties and signals: harbour.threesea's name ->
# (header, class).
ITEMS = {
    "AnimatedMedia": ("animatedimage.h", "AnimatedImage"),
    "CallVideo": ("callvideo.h", "CallVideo"),
}

QML_TYPES = {
    "QString": "string",
    "bool": "bool",
    "int": "int",
    "uint": "int",
    "qreal": "real",
    "double": "real",
    "float": "real",
    "qint64": "real",
    "quint64": "real",
}
DEFAULTS = {"string": '""', "bool": "false", "int": "0", "real": "0"}


def values(flavour):
    full = "true" if flavour == "full" else "false"
    return {
        "backend": {
            "state": '"starting"',
            "identity": '"AAAAAAAA"',
            "connectionState": '"connected"',
            "contacts": "contactsModel",
            "full": full,
            "emulator": "true",
        },
        "calls": {"supported": full, "state": '"idle"'},
        "groupCalls": {"supported": full, "state": '"idle"'},
        "voicePlayer": {"messageId": "-1", "speed": "1"},
        "videoPlayer": {"messageId": "-1"},
    }


# Set by the test while it runs, whatever the real class allows QML to do.
CHANGING = {"backend": {"state", "unlockedAtStart"}}

# Functions the fakes answer in a certain way; the rest return an empty value of their type.
FUNCTIONS = {
    "backend": {
        # After a moment, like the real one, with what Results.js has for the command.
        "call": """function call(cmd, args, callback) {
            self.waiting.push({ cmd: cmd, args: args, callback: callback })
        }""",
        "chatTone": 'function chatTone() { return "" }',
        "captureFile": 'function captureFile(extension) { return "/cache/capture/test." + extension }',
    },
    "dbus": {"actionToken": 'function actionToken() { return "token" }'},
    "groupCalls": {"call": "function call(conversation) { return ({}) }"},
}
EXTRA = {
    "backend": """
        id: self
        property var waiting: []
        property var replyTimer: Timer {
            interval: 1
            repeat: true
            running: true
            onTriggered: {
                var now = self.waiting
                self.waiting = []
                for (var i = 0; i < now.length; ++i) {
                    if (now[i].callback) now[i].callback(true, Results.answer(now[i].cmd, now[i].args))
                }
            }
        }
""",
}


def preprocess(text, flavour):
    """Comments away, and only the flavour's side of its #ifdefs (and none of the demo's)."""
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    text = re.sub(r"//[^\n]*", "", text)
    kept = []
    stack = []
    for line in text.split("\n"):
        word = line.strip()
        match = re.match(r"#\s*(ifdef|ifndef|if|else|elif|endif)\b\s*(\S*)", word)
        if match:
            kind, name = match.groups()
            if kind in ("ifdef", "ifndef", "if"):
                on = True
                if name == "THREESEA_FULL":
                    on = (flavour == "full") == (kind == "ifdef")
                elif name == "THREESEA_DEMO":
                    on = kind == "ifndef"
                elif name == "Q_PROCESSOR_X86_32":
                    on = (kind == "ifdef")
                stack.append(on)
            elif kind == "else" and stack:
                stack[-1] = not stack[-1]
            elif kind == "endif" and stack:
                stack.pop()
            continue
        if all(stack):
            kept.append(line)
    return "\n".join(kept)


def class_body(text, name):
    match = re.search(r"\bclass\s+" + name + r"\b[^;{]*\{", text)
    if not match:
        sys.exit("no class %s" % name)
    depth, at = 1, match.end()
    while depth:
        depth += {"{": 1, "}": -1}.get(text[at], 0)
        at += 1
    return text[match.end():at - 1]


def qml_type(cpp):
    cpp = cpp.replace("const", "").replace("&", "").strip()
    return QML_TYPES.get(cpp, "var")


def empty_value(cpp):
    kind = qml_type(cpp)
    if kind != "var":
        return DEFAULTS[kind]
    return "[]" if "List" in cpp else "({})" if "Map" in cpp else "null"


def params(text):
    """The QML names and types of a C++ parameter list."""
    depth, parts, current = 0, [], ""
    for char in text:
        depth += {"<": 1, "(": 1, "{": 1, ">": -1, ")": -1, "}": -1}.get(char, 0)
        if char == "," and depth == 0:
            parts.append(current)
            current = ""
        else:
            current += char
    if current.strip():
        parts.append(current)
    result = []
    for index, part in enumerate(parts):
        part = part.split("=")[0].strip()
        match = re.match(r"(.*?[\s&*>])(\w+)$", part)
        if match and match.group(1).replace("const", "").strip():
            result.append((qml_type(match.group(1)), match.group(2)))
        else:
            result.append((qml_type(part), "arg%d" % index))
    return result


def functions_in(section):
    """(return type, name, parameters) of the declarations and inline definitions in `section`."""
    found = []
    for match in re.finditer(r"([\w:<>,\s\*&]+?)\s+\*?&?(\w+)\s*\(([^()]*(?:\([^()]*\)[^()]*)*)\)", section):
        kind = match.group(1).split(";")[-1].split("}")[-1].strip()
        kind = re.sub(r"\b(virtual|static|inline|explicit)\b", "", kind).strip()
        if kind and not kind.startswith(("return", "if ", "emit")):
            found.append((kind, match.group(2), match.group(3)))
    return found


def parse(path, name, flavour):
    body = class_body(preprocess(open(path, encoding="utf-8").read(), flavour), name)
    properties = {}
    for match in re.finditer(r"Q_PROPERTY\(\s*(.+?)\s+\*?(\w+)\s+((?:READ|MEMBER)\b.*?)\)\s*\n", body, re.S):
        writable = bool(re.search(r"\b(WRITE|MEMBER)\b", match.group(3)))
        properties[match.group(2)] = (qml_type(match.group(1)), writable, empty_value(match.group(1)))
    invokable, signals = {}, {}
    for match in re.finditer(r"Q_INVOKABLE\s+([^;{]+?\))\s*(?:const)?\s*[;{]", body, re.S):
        for kind, function, args in functions_in(match.group(1)):
            invokable.setdefault(function, (kind, args))
    sections = re.split(r"\n\s*(public|protected|private)?\s*(slots|signals|Q_SLOTS|Q_SIGNALS)?\s*:(?!:)", body)
    # re.split leaves [text, access, kind, text, access, kind, ...].
    for at in range(1, len(sections) - 2, 3):
        access, kind, text = sections[at], sections[at + 1], sections[at + 2]
        if kind in ("signals", "Q_SIGNALS"):
            for _, function, args in functions_in(text):
                signals.setdefault(function, args)
        elif access == "public" and kind in ("slots", "Q_SLOTS"):
            for result, function, args in functions_in(text):
                invokable.setdefault(function, (result, args))
    return properties, invokable, signals


def members(properties, invokable, signals, given, changing, answers, indent):
    lines = []
    for name, (kind, writable, empty) in sorted(properties.items()):
        value = given.get(name, empty)
        readonly = "" if writable or name in changing else "readonly "
        lines.append("%s%sproperty %s %s: %s" % (indent, readonly, kind, name, value))
    automatic = {name + "Changed" for name in properties}
    for name, args in sorted(signals.items()):
        if name in automatic:
            continue
        lines.append("%ssignal %s(%s)" % (indent, name, ", ".join("%s %s" % p for p in params(args))))
    for name, (result, args) in sorted(invokable.items()):
        if name in properties:
            continue
        if name in answers:
            lines.append(indent + answers[name])
            continue
        names = ", ".join(p[1] for p in params(args))
        if result.strip() == "void":
            lines.append("%sfunction %s(%s) { }" % (indent, name, names))
        else:
            lines.append("%sfunction %s(%s) { return %s }" % (indent, name, names, empty_value(result)))
    return "\n".join(lines)


def check(bridge, flavour, qml):
    """What the QML asks of the C++ objects and they don't have (QObject's own aside)."""
    found = 0
    for root, _, files in os.walk(qml):
        for file in files:
            if not file.endswith((".qml", ".js")):
                continue
            path = os.path.join(root, file)
            text = preprocess(open(path, encoding="utf-8").read().replace("#", " "), flavour)
            for name, (header, cls) in OBJECTS.items():
                properties, invokable, signals = parse(os.path.join(bridge, header), cls, flavour)
                known = set(properties) | set(invokable) | set(signals) | {"objectName"}
                for match in re.finditer(r"(?<![\w.])" + name + r"\.(\w+)", text):
                    if match.group(1) not in known:
                        print("%s: %s has no %s" % (os.path.relpath(path, qml), name, match.group(1)))
                        found += 1
    return found


def main():
    if sys.argv[1] == "--check":
        sys.exit(1 if check(*sys.argv[2:5]) else 0)
    bridge, flavour, out = sys.argv[1:4]
    given = values(flavour)
    parts = ['// Generated by make-fakes.py from the bridge\'s headers.\nimport QtQuick 2.6\n'
             'import "Results.js" as Results\n\nQtObject {']
    for name, (header, cls) in OBJECTS.items():
        parsed = parse(os.path.join(bridge, header), cls, flavour)
        parts.append("    property QtObject %s: QtObject {" % name)
        parts.append(EXTRA.get(name, "").rstrip("\n"))
        parts.append(members(*parsed, given.get(name, {}), CHANGING.get(name, set()),
                             FUNCTIONS.get(name, {}), " " * 8))
        parts.append("    }")
    parts.append("}\n")
    with open(os.path.join(out, "Fakes.qml"), "w", encoding="utf-8") as output:
        output.write("\n".join(part for part in parts if part))
    for name, (header, cls) in ITEMS.items():
        properties, invokable, signals = parse(os.path.join(bridge, header), cls, flavour)
        with open(os.path.join(out, name + ".qml"), "w", encoding="utf-8") as output:
            output.write("// Generated by make-fakes.py from the bridge's headers.\n"
                         "import QtQuick 2.6\n\nItem {\n%s\n}\n"
                         % members(properties, invokable, signals, {}, set(), {}, " " * 4))


if __name__ == "__main__":
    main()
