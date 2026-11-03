#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""CLAUDE.md §7.3: logs carry fixed texts plus kinds, codes, counts and internal IDs; never
content, keys, PINs, phone numbers, e-mail addresses or text from outside verbatim.

Checks every log call (Rust `tracing::…!`, C++ `qWarning()/qInfo()/qDebug()/qCritical()`,
QML/JS `console.…`): what it logs besides string literals must not be named like content or
personal data. A deliberate exception carries `log-ok: <why>` in a comment on the call's first
line. Crude on purpose: it catches the usual slip when it is made. Exit status 1 on findings."""
import re
import subprocess
import sys

DENY = re.compile(
    r"\b(text|body|caption|content|plain\w*|pin|passphrase|password|secret|key|keys|pcmk|"
    r"email|e_mail|mail|phone|mobile|msisdn|number|nickname|name|display_?name|first_?name|"
    r"last_?name|identity|threema_?id|sender|recipient|peer|contact|reason|sdp|candidate|"
    r"payload|token|nonce|address|location|description|title|caption|server_?response)\b",
    re.IGNORECASE,
)
RUST_CALL = re.compile(r"\btracing::(?:trace|debug|info|warn|error)!\(")
# Stream style `qWarning() << …` and printf style `qWarning("…", value)`.
CPP_CALL = re.compile(r"\bq(?:Warning|Info|Debug|Critical)\(")
# Variables captured inside a Rust format string: `"… {body}"`, `"{body:?}"`.
CAPTURED = re.compile(r"\{([A-Za-z_][A-Za-z0-9_.]*)(?::[^}]*)?\}")
JS_CALL = re.compile(r"\bconsole\.(?:log|info|warn|error|debug)\(")
STRING = re.compile(r'"(?:[^"\\]|\\.)*"' + r"|'(?:[^'\\]|\\.)*'")


def tracked(patterns):
    # Also new files not added yet.
    out = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard", *patterns],
                         capture_output=True, text=True, check=True)
    return [path for path in out.stdout.split() if not path.startswith("third_party/")]


def balanced(source, start):
    """The text from `start` (just after an opening parenthesis) to its closing one."""
    depth = 1
    i = start
    while i < len(source) and depth:
        char = source[i]
        if char in "\"'":
            quote = char
            i += 1
            while i < len(source) and source[i] != quote:
                i += 2 if source[i] == "\\" else 1
        elif char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
        i += 1
    return source[start:i - 1]


def calls(path, source):
    if path.endswith(".rs"):
        for match in RUST_CALL.finditer(source):
            yield match.start(), balanced(source, match.end())
    elif path.endswith((".cpp", ".h")):
        for match in CPP_CALL.finditer(source):
            # The call's own arguments, then any `<< …` up to the end of the statement.
            arguments = balanced(source, match.end())
            end = source.find(";", match.end())
            yield match.start(), arguments + " " + source[match.end() + len(arguments) + 1:end]
    else:
        for match in JS_CALL.finditer(source):
            yield match.start(), balanced(source, match.end())


def check(path, source):
    """The findings in one file."""
    findings = []
    for offset, arguments in calls(path, source):
        line_start = source.rfind("\n", 0, offset) + 1
        line_end = source.find("\n", offset)
        first_line = source[line_start:line_end]
        previous = source[source.rfind("\n", 0, line_start - 1) + 1:line_start]
        if "log-ok:" in first_line or "log-ok:" in previous:
            continue
        captured = " ".join(CAPTURED.findall(" ".join(STRING.findall(arguments))))
        code = STRING.sub("", arguments) + " " + captured
        # Field names in `tracing` calls (`kind = …`) say what is logged as much as values.
        hits = sorted({match.group(0) for match in DENY.finditer(code)})
        if hits:
            line = source.count("\n", 0, offset) + 1
            findings.append(f"{path}:{line}: logs {', '.join(hits)}: {first_line.strip()}")
    return findings


# Calls the checker must flag (deep audit 2026-10-27, D05: captured format arguments,
# printf-style Qt calls) and ones it must let pass.
SELF_TEST_FLAGGED = [
    ("a.rs", 'tracing::info!("private text: {body}");'),
    ("a.rs", 'tracing::warn!(%reason, "closed");'),
    ("a.rs", 'tracing::info!(name = contact.name, "added");'),
    ("a.cpp", 'qWarning("failed: %s", qPrintable(text));'),
    ("a.cpp", 'qWarning() << "peer" << peer;'),
    ("a.qml", 'console.log("message", model.body)'),
]
SELF_TEST_PASSED = [
    ("a.rs", 'tracing::info!(count = 3, "done");'),
    ("a.rs", 'tracing::warn!(kind = error.kind(), "failed");'),
    ("a.cpp", 'qWarning() << "frames:" << frames;'),
    ("a.rs", '// log-ok: a code\ntracing::warn!(%reason, "closed");'),
]


def self_test():
    for path, source in SELF_TEST_FLAGGED:
        assert check(path, source), f"not flagged: {source}"
    for path, source in SELF_TEST_PASSED:
        assert not check(path, source.replace("\\n", "\n")), f"flagged: {source}"


def main():
    self_test()
    findings = []
    for path in tracked(["*.rs", "*.cpp", "*.h", "*.qml", "*.js"]):
        with open(path, encoding="utf-8") as file:
            findings += check(path, file.read())
    for finding in findings:
        print(finding)
    if findings:
        print(f"\n{len(findings)} log call(s) may log content or personal data (CLAUDE.md §7.3); "
              "fix them or mark a deliberate one with `log-ok: <why>`.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
