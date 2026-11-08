// SPDX-License-Identifier: AGPL-3.0-only
#include "installcheck.h"

#include <QtGlobal>

#include <cerrno>
#include <sys/stat.h>

InstallCheck::InstallCheck(const QByteArray &path)
    : m_path(path)
{
    struct stat now;
    if (::stat(m_path.constData(), &now) == 0) {
        m_known = true;
        m_device = now.st_dev;
        m_inode = now.st_ino;
        m_size = now.st_size;
        m_modified = now.st_mtime;
    } else {
        qWarning("threesea: the package's main file can't be looked at (%d): updates go unnoticed", errno);
    }
}

InstallCheck::State InstallCheck::state() const
{
    if (!m_known) {
        return Current;
    }
    struct stat now;
    if (::stat(m_path.constData(), &now) != 0) {
        return errno == ENOENT || errno == ENOTDIR ? Removed : Current;
    }
    // A package manager can leave a file it finds unchanged in place, with the package's time.
    const bool same = now.st_dev == m_device && now.st_ino == m_inode && now.st_size == m_size
        && now.st_mtime == m_modified;
    return same ? Current : Replaced;
}
