# 0001 — Thin C++ bridge over a C ABI (not qmetaobject-rs / cxx-qt)

**Context.** Whisperfish (Signal for Sailfish) exposes Rust to QML via `qmetaobject-rs`. cxx-qt
needs Qt ≥ 5.15/6; Sailfish ships Qt **5.6.3**. libthreema's state machines use `Rc`/`RefCell`
and are not `Send`.

**Decision.** Keep the stack from CLAUDE.md §2: Rust core exports a small C ABI
(`core/include/threesea_core.h`), a thin C++/Qt 5.6 layer (`bridge/`) owns models, D-Bus and
platform APIs. The Rust core runs its own single event-loop thread; C++ only marshals.

**Consequences.** Platform APIs that are C++/QML only (Sailfish Secrets, Nemo notifications,
KeepAlive, QtMultimedia) are trivially reachable. More boilerplate per FFI call than
qmetaobject-rs; mitigate with a message/event-based FFI (few entry points, serialized payloads)
instead of one C function per feature. Revisit only with a written reason.
