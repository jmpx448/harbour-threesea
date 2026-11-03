#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# CI only (GitHub's Ubuntu runners): make room for the Sailfish SDK image (about 16 GB) and
# fetch its base image by digest, tagged as scripts/sdk/Dockerfile expects it, so a build always
# starts from exactly this image (security audit 2026-10-27: pin build inputs).
set -eu
SFOS=${SFOS_VERSION:-5.1.0.11}
BASE=coderus/sailfishos-platform-sdk
# docker image inspect $BASE:5.1.0.11 --format '{{json .RepoDigests}}'
DIGEST=sha256:31ba115546972af3630f4690acc204a4e09701358ae39be1883ca70e0eadda86

# The runner's preinstalled toolchains we don't use.
sudo rm -rf /usr/share/dotnet /usr/local/lib/android /opt/ghc /usr/local/.ghcup \
    /opt/hostedtoolcache/CodeQL /usr/local/share/boost /usr/share/swift
sudo docker image prune -a -f >/dev/null
df -h / | tail -1

docker pull -q "$BASE@$DIGEST"
docker tag "$BASE@$DIGEST" "$BASE:$SFOS"
