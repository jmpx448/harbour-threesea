// SPDX-License-Identifier: AGPL-3.0-only
#ifndef INSTALLCHECK_H
#define INSTALLCHECK_H

#include <QByteArray>

#include <sys/types.h>

// Whether the installed package still is the one this process came from: an update replaces its
// files, a removal takes them away, and the process runs on regardless. The Lite flavour has no
// package scripts to end it (Harbour restricts them), and the Full flavour's only act at the end
// of the transaction.
//
// It looks at a file of the package under /usr/share, not the executable: Sailjail starts the app
// with --private-bin, so inside the sandbox /usr/bin/<app> is a copy that never changes, whereas
// /usr/share/<app> is the real directory.
class InstallCheck
{
public:
    enum State {
        Current,
        // Another file is at the path: a new version, whose QML the pages now load from disk.
        Replaced,
        Removed,
    };

    // What is at `path` now counts as this process's own.
    explicit InstallCheck(const QByteArray &path);

    // If nothing can be said (the path could not be looked at from the start, or only now and for
    // other reasons than its absence) the answer is Current: this ends the process, so only a
    // sure thing may do that.
    State state() const;

private:
    QByteArray m_path;
    bool m_known = false;
    dev_t m_device = 0;
    ino_t m_inode = 0;
    off_t m_size = 0;
    time_t m_modified = 0;
};

#endif // INSTALLCHECK_H
