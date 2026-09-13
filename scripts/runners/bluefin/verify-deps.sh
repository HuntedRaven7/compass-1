#!/usr/bin/env bash
# Assert the headers and libraries the C++ tree consumes DIRECTLY are present.
#
# WHY THIS EXISTS
#
# The configure job cannot catch these. `cmake` only resolves what the tree asks
# for through find_package, and this tree also consumes dependencies two ways
# CMake never checks:
#
#   - bare link names, e.g. src/server/CMakeLists.txt:990
#       list(APPEND LIBS xkbcommon xkbcommon-x11)
#     which become -lxkbcommon at link time and are not looked for at all
#     until then;
#   - bare #includes of system headers with no find_package behind them.
#
# So a missing libxkbcommon-devel configures perfectly and then fails at object
# 624 of 812, eighteen minutes in. That is exactly what happened, and adding one
# package per red run is the pattern that already cost four cycles on the
# configure job. This checks the whole set in about a second.
#
# HOW THE LIST WAS DERIVED, so it can be re-derived rather than guessed:
#
#   headers: grep the tree for namespaced system #includes, drop Qt, KF6, the
#            vendored trees (xdgpp, sqlcipher, zip, pugixml, cmark-gfm), the
#            Windows ones (winrt/, wrl/) and Catch2, which the Bluefin builds
#            disable.
#   libs:    grep target_link_libraries for names that are neither CMake targets
#            (they contain no ::) nor vendored.
#
# Qt itself is deliberately absent: configure already proves every Qt component
# resolves, and probing Qt headers here would need the include paths CMake
# computes.
set -euo pipefail

status=0

# check_header <header> [prelude line...]
#
# The prelude matters, and its absence made this script's first run report a
# false negative. A probe that says MISSING for a header that is installed is
# worse than no probe at all: it blocks a build that would have worked, and it
# points at the wrong fix. So a header is probed the way the TREE includes it,
# not in isolation.
check_header() {
    local header="$1"
    shift
    local src=""
    local line
    for line in "$@"; do
        src+="$line"$'\n'
    done
    src+="#include <$header>"$'\n'

    if printf '%s' "$src" | c++ -fsyntax-only -x c++ - 2>/dev/null; then
        echo "  header ok      $header"
    else
        echo "  header MISSING $header"
        status=1
    fi
}

check_lib() {
    if echo 'int main(){return 0;}' | c++ -x c++ - "-l$1" -o /dev/null 2>/dev/null; then
        echo "  lib    ok      -l$1"
    else
        echo "  lib    MISSING -l$1"
        status=1
    fi
}

echo "Headers the tree includes directly:"
check_header xkbcommon/xkbcommon.h
check_header xkbcommon/xkbcommon-keysyms.h
check_header xkbcommon/xkbcommon-x11.h
check_header libudev.h
check_header wayland-client.h
check_header xcb/xcb.h
check_header xcb/xproto.h
check_header xcb/xcb_keysyms.h
# xcb/xkb.h declares a field called `explicit`, so it does not compile as C++
# on its own. src/server/src/internal/keyboard/x11-layout-resolver.cpp:3-6
# includes xcb/xcb.h and then #defines the keyword away before including it,
# and this probe has to do the same. Probing it bare reported MISSING against a
# Bluefin image that HAD the header — the previous full build compiled that
# same file past line 6 and failed at line 8, which is how the false negative
# was caught.
check_header xcb/xkb.h '#include <xcb/xcb.h>' '#define explicit explicit_'
check_header openssl/evp.h
check_header openssl/kdf.h
check_header openssl/rand.h
check_header libqalculate/Calculator.h

echo "Libraries the tree links by bare name:"
check_lib xkbcommon
check_lib xkbcommon-x11
check_lib udev
check_lib wayland-client

if [ "$status" -ne 0 ]; then
    echo "::error::a dependency the tree uses directly is not installed — see above"
fi
exit "$status"
