#!/usr/bin/env bash
# Build dependencies for the C++ engine on Arch.
#
# Single source of truth, used by both the container image
# (scripts/runners/arch/base.Dockerfile) and CI jobs that run directly on
# archlinux:latest. Keep it that way: duplicating this list into a workflow is
# how the two silently drift.
set -euo pipefail

pacman -Syu --needed --noconfirm \
    curl jq git base-devel gcc clang cmake ninja mold ccache \
    nodejs npm \
    qt6-base qt6-svg qt6-declarative qt6-shadertools qt6-tools \
    qtkeychain-qt6 layer-shell-qt syntax-highlighting extra-cmake-modules \
    libqalculate \
    catch2 wayland-protocols

pacman -Scc --noconfirm
