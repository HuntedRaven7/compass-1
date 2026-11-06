# The Rust engine, installed the way the Flatpak installs it.
#
# `nix build .#compass` (alias `.#rust-vicinae`). The layout comes from
# scripts/packaging/install-rust-engine.sh, shared with the AppImage and the
# Arch package, and scripts/packaging/smoke.sh checks it in CI (Suite 5).
#
# The whole repository is the source, not a filter of Cargo files: the build
# reads SQL migrations, the extension boilerplate, vendor/fuzzy-trigram's C and
# the C++ glyph and icon tables through include_str! and build.rs, and a
# Cargo-only filter silently dropped them.
{
  lib,
  craneLib,
  src,
  extensionRuntime,
  pkg-config,
  makeWrapper,
  patchelf,
  nodejs,
  openssl,
  wayland,
  libxkbcommon,
  vulkan-loader,
  libGL,
}: let
  # winit and wgpu dlopen these rather than linking them, so fixup's
  # --shrink-rpath would strip them; they are added back after it, below.
  dlopened = lib.makeLibraryPath [
    wayland
    libxkbcommon
    vulkan-loader
    libGL
  ];

  # Each vendored crate is its own derivation whose $out is the crate root, so
  # stdenv's fixup (move-docs.sh) moves a top-level doc/ to share/doc and
  # breaks crates that include_str! from it (bitvec, under evdev). Vendored
  # crates are sources; nothing in them needs fixing up.
  cargoVendorDir = craneLib.vendorCargoDeps {
    inherit src;
    overrideVendorCargoPackage = _: drv: drv.overrideAttrs (_: {dontFixup = true;});
  };

  commonArgs = {
    pname = "compass";
    version = "0.28.4";
    inherit src cargoVendorDir;

    strictDeps = true;
    # compass-input-server: the keyboard helper; the Nix store cannot carry
    # its capability, so NixOS wraps it (security.wrappers, packaging/README.md).
    cargoExtraArgs = "--locked -p compass -p compass-sandbox -p compass-input-server --bins";
    doCheck = false;

    nativeBuildInputs = [
      pkg-config
      makeWrapper
      patchelf
    ];
    # libsqlite3-sys's bundled SQLCipher links libcrypto; iced_layershell's
    # smithay-client-toolkit needs xkbcommon at build time.
    buildInputs = [
      openssl
      libxkbcommon
    ];
    OPENSSL_LIB_DIR = "${lib.getLib openssl}/lib";
    OPENSSL_INCLUDE_DIR = "${lib.getDev openssl}/include";
  };

  # Built on its own so the install and fixup below apply to the package
  # only: the dependencies-only build has no bin/compass to patch.
  #
  # The dependencies-only build swaps every Rust source for a stub, and that
  # includes the patched crates in vendor/ that `[patch.crates-io]` points at;
  # crates.io dependencies built against those stubs fail. They are put back.
  cargoArtifacts = craneLib.buildDepsOnly (commonArgs
    // {
      dummySrc = craneLib.mkDummySrc {
        inherit src;
        extraDummyScript = ''
          rm -rf $out/vendor/layershellev $out/vendor/smithay-clipboard
          cp -r ${src}/vendor/layershellev ${src}/vendor/smithay-clipboard $out/vendor/
        '';
      };
    });
in
  craneLib.buildPackage (commonArgs
    // {
      inherit cargoArtifacts;

      installPhaseCommand = ''
        PREFIX="$out" LIBEXECDIR="$out/libexec" BIN_DIR="$PWD/target/release" \
          RUNTIME_JS=${extensionRuntime}/share/compass/extension-runtime.js REQUIRE_RUNTIME=1 \
          bash scripts/packaging/install-rust-engine.sh
      '';

      # COMPASS_NODE rather than Node on PATH: the launcher passes its
      # environment to every application it starts, and they should not inherit
      # our Node. COMPASS_BUILTIN_ICONS because `nix run` puts nothing on
      # XDG_DATA_DIRS. The helpers, the runtime bundle and the schema are found
      # relative to the binary, which the wrapper keeps in $out/bin.
      postFixup = ''
        patchelf --add-rpath ${dlopened} "$out/bin/compass"
        wrapProgram "$out/bin/compass" \
          --set-default COMPASS_NODE ${lib.getExe nodejs} \
          --set-default COMPASS_BUILTIN_ICONS "$out/share/compass/builtin-icons"
      '';

      meta = {
        description = "Compass, a fast, extensible command palette for Linux";
        homepage = "https://github.com/tuna-os/compass";
        license = lib.licenses.gpl3Only;
        platforms = lib.platforms.linux;
        mainProgram = "compass";
      };
    })
