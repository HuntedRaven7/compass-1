#!/usr/bin/env bash
# Build dependencies for the C++ engine on the Bluefin target.
#
# Single source of truth, for the same reason the Arch script next door is one:
# .github/workflows/cpp-on-target.yaml runs configure and the full build as two
# jobs, and a list duplicated between them is a list that drifts.
#
# This is Arch's list translated, and the translation is not one-to-one:
#
#   qt6-qtbase-private-devel   Arch ships private headers inside qt6-base;
#                              Fedora splits them out, and src/server links
#                              Qt6::GuiPrivate.
#   layer-shell-qt-devel       Required, not optional. USE_SYSTEM_LAYER_SHELL
#                              is ON unconditionally (CMakeLists.txt:53) — the
#                              FetchContent path is reached only under
#                              PREFER_STATIC_LIBS, which is the AppImage build.
#   libsecret-devel            Fedora has no Qt6 qtkeychain, so the build uses
#                              -DUSE_SYSTEM_QT_KEYCHAIN=OFF and the vendored
#                              copy still links system libsecret. The option's
#                              own help text says so.
#   no catch2                  Fedora 44 ships 2.13.10; the tests require
#                              Catch2 3, which Fedora has no package for. The
#                              builds here pass -DBUILD_TESTS=OFF, and that is
#                              the right answer rather than a workaround: Suite
#                              0 diffs engine behaviour through the CLI, and the
#                              C++ unit tests already run on Arch where Catch2
#                              is v3.
#
# The rest was enumerated from the tree's own find_package calls — ECM,
# KF6SyntaxHighlighting, LayerShellQt, OpenSSL, PkgConfig and X11 REQUIRED with
# the xcb, xcb_keysyms and xcb_xkb components — rather than discovered one red
# CI run at a time, which is how the first four revisions of this list were
# found and is a poor way to spend cycles even cheap ones.
#
# THAT ENUMERATION WAS STILL INCOMPLETE, and a full build proved it at object
# 624 of 812:
#
#   fatal error: xkbcommon/xkbcommon-x11.h: No such file or directory
#
# find_package is not the whole story. This tree also links by bare name —
# `list(APPEND LIBS xkbcommon xkbcommon-x11)` at src/server/CMakeLists.txt:990,
# and `xkbcommon udev` at src/snippet/CMakeLists.txt:28 — which CMake never
# looks for at configure time, so a missing package configures cleanly and
# fails eighteen minutes into the compile. libxkbcommon-devel,
# libxkbcommon-x11-devel and systemd-devel (for libudev) cover those; udev was
# found by the same sweep and would otherwise have been the next red run.
#
# verify-deps.sh next door checks that whole class in about a second, and both
# jobs run it. Read its header before adding anything here.
set -euo pipefail

dnf install -y --setopt=install_weak_deps=False \
    gcc gcc-c++ cmake ninja-build git \
    qt6-qtbase-devel qt6-qtbase-private-devel \
    qt6-qtsvg-devel qt6-qtdeclarative-devel \
    qt6-qtshadertools-devel qt6-qttools-devel \
    qt6-qtwayland-devel \
    kf6-syntax-highlighting-devel extra-cmake-modules \
    layer-shell-qt-devel \
    libqalculate-devel \
    wayland-devel wayland-protocols-devel \
    libxkbcommon-devel libxkbcommon-x11-devel \
    systemd-devel \
    libsecret-devel openssl-devel \
    libX11-devel libxcb-devel xcb-util-keysyms-devel \
    nodejs npm
