#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""CLAUDE.md §7.3: logs carry fixed texts plus kinds, codes, counts and internal IDs; never
content, keys, PINs, phone numbers, e-mail addresses or text from outside verbatim.

Checks every log call (Rust `tracing::…!`, also unqualified as libthreema writes them, spans and
`#[tracing::instrument]`, whose fields are printed with every event inside them, C++
`qWarning()/qInfo()/qDebug()/qCritical()`, QML/JS `console.…`): what it logs besides string
literals must not be named like content or personal data. Our code and the part of libthreema
linked into the app are checked (not its CLI, bindings, or in-memory test stores). A deliberate
exception carries `log-ok: <why>` in a comment on the call's first line or the line before.
Crude on purpose: it catches the usual slip when it is made. Exit status 1 on findings."""
import re
import subprocess
import sys

DENY = re.compile(
    r"\b(text|body|caption|content|plain\w*|pin|passphrase|password|secret|key|keys|pcmk|"
    r"email|e_mail|mail|phone|mobile|msisdn|number|nickname|name|display_?name|first_?name|"
    r"last_?name|\w*identity|identities|threema_?id|sender|recipient|peer|contact|contacts|"
    r"reason|sdp|candidate|"
    r"payload|token|nonce|address|location|description|title|caption|server_?response)\b",
    re.IGNORECASE,
)
RUST_CALL = re.compile(
    r"(?<![\w:])(?:tracing::)?(?:trace|debug|info|warn|error)(?:_span)?!\s*[({]"
    r"|#\[tracing::instrument\("
)
# Stream style `qWarning() << …` and printf style `qWarning("…", value)`.
CPP_CALL = re.compile(r"\bq(?:Warning|Info|Debug|Critical)\(")
# Variables captured inside a Rust format string: `"… {body}"`, `"{body:?}"`.
CAPTURED = re.compile(r"\{([A-Za-z_][A-Za-z0-9_.]*)(?::[^}]*)?\}")
JS_CALL = re.compile(r"\bconsole\.(?:log|info|warn|error|debug)\(")
# A count, or an internal ID: `contacts = ids.len()`, `id = ?payload.id`.
NOT_CONTENT = re.compile(r"\b\w+\s*=\s*[?%]?[\w.]+\.(?:len\(\)|count\(\)|id)(?!\w)")
STRING = re.compile(r'"(?:[^"\\]|\\.)*"' + r"|'(?:[^'\\]|\\.)*'")


LIBTHREEMA = "third_party/libthreema/lib/src/"
LIBTHREEMA_NOT_IN_THE_APP = tuple(LIBTHREEMA + part for part in ("cli/", "bindings/"))
# Compiled only for tests and the CLI (`cfg(any(test, feature = "cli"))`, in_memory).
TEST_STORE_FROM = {LIBTHREEMA + "model/provider.rs": "pub mod in_memory {"}


def tracked(patterns):
    # Also new files not added yet.
    out = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard", *patterns],
                         capture_output=True, text=True, check=True)
    return [path for path in out.stdout.split() if checked(path)]


def checked(path):
    """Ours, and the part of libthreema that runs in the app (its logs land in the same places)."""
    if path.startswith(LIBTHREEMA):
        return not path.startswith(LIBTHREEMA_NOT_IN_THE_APP) and "/tests" not in path
    return not path.startswith("third_party/")


def balanced(source, start, opening="(", closing=")"):
    """The text from `start` (just after an opening bracket) to its closing one."""
    depth = 1
    i = start
    while i < len(source) and depth:
        char = source[i]
        if char in "\"'":
            quote = char
            i += 1
            while i < len(source) and source[i] != quote:
                i += 2 if source[i] == "\\" else 1
        elif char == opening:
            depth += 1
        elif char == closing:
            depth -= 1
        i += 1
    return source[start:i - 1]


def calls(path, source):
    if path.endswith(".rs"):
        for match in RUST_CALL.finditer(source):
            brace = source[match.end() - 1] == "{"
            yield match.start(), balanced(source, match.end(), *("{}" if brace else "()"))
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
    source = source[:source.find(TEST_STORE_FROM[path])] if path in TEST_STORE_FROM else source
    for offset, arguments in calls(path, source):
        line_start = source.rfind("\n", 0, offset) + 1
        line_end = source.find("\n", offset)
        first_line = source[line_start:line_end]
        previous = source[source.rfind("\n", 0, line_start - 1) + 1:line_start]
        if "log-ok:" in first_line or "log-ok:" in previous:
            continue
        captured = " ".join(CAPTURED.findall(" ".join(STRING.findall(arguments))))
        code = STRING.sub("", arguments) + " " + captured
        code = NOT_CONTENT.sub("", code)
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
    # libthreema's style: macros imported, span fields printed with every event inside them.
    ("a.rs", 'warn!(?identity, "Discarding");'),
    ("a.rs", 'warn!(name, "Discarding");'),
    ("a.rs", 'warn!(sender_identity = ?message.sender_identity, "Discarding");'),
    ("a.rs", 'info!(contacts = ?identities, "Added");'),
    ("a.rs", 'trace!(?payload, "Decoded");'),
    ("a.rs", 'let _span = tracing::info_span! { "message", sender = ?payload.sender };'),
    ("a.rs", '#[tracing::instrument(skip_all, fields(?payload))]'),
]
SELF_TEST_PASSED = [
    ("a.rs", 'tracing::info!(count = 3, "done");'),
    ("a.rs", 'tracing::warn!(kind = error.kind(), "failed");'),
    ("a.cpp", 'qWarning() << "frames:" << frames;'),
    ("a.rs", '// log-ok: a code\ntracing::warn!(%reason, "closed");'),
    ("a.rs", 'warn!(?session_id, "Rejecting");'),
    ("a.rs", 'info!(contacts = ids.len(), "refreshed");'),
    ("a.rs", 'trace!(id = ?payload.id, "Decoded");'),
    ("a.rs", 'let _span = tracing::info_span! { "message", id = ?session.id };'),
    ("a.rs", '#[tracing::instrument(skip_all, fields(?self))]'),
    # Not log macros.
    ("a.rs", 'my_warn!(name);'),
    ("a.rs", 'other::warn!(name);'),
]
# What decides what is scanned.
SELF_TEST_CHECKED = ["core/src/app.rs", LIBTHREEMA + "csp_e2e/message/codec.rs"]
SELF_TEST_SKIPPED = [
    LIBTHREEMA + "cli/mod.rs",
    LIBTHREEMA + "bindings/uniffi/mod.rs",
    "third_party/libthreema/lib/tests/x.rs",
    "third_party/opus-pure/src/lib.rs",
]


def self_test():
    for path, source in SELF_TEST_FLAGGED:
        assert check(path, source), f"not flagged: {source}"
    for path, source in SELF_TEST_PASSED:
        assert not check(path, source.replace("\\n", "\n")), f"flagged: {source}"
    for path in SELF_TEST_CHECKED:
        assert checked(path), f"skipped: {path}"
    for path in SELF_TEST_SKIPPED:
        assert not checked(path), f"scanned: {path}"


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
