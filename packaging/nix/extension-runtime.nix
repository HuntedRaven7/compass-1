# The extension runtime bundle (extension-runtime.js) for the Rust engine.
#
# Built by the same scripts/build-extension-runtime.sh the Flatpak workflow and
# the Arch package run: npm bundles the committed protocol bindings. The npm
# dependency hashes live here; `nix run .#nix-update-script` checks them and
# scripts/update-nix-npm-hashes.sh rewrites them.
{
  lib,
  stdenv,
  fetchNpmDeps,
  nodejs,
  npmHooks,
  src,
}: let
  apiDeps = fetchNpmDeps {
    src = "${src}/src/typescript/api";
    hash = "sha256-4FEaBDJK9abcgz+vptuL4wQ8zhp+wpLbbR4Y79BVhEg=";
  };

  extensionManagerDeps = fetchNpmDeps {
    src = "${src}/src/typescript/extension-manager";
    hash = "sha256-pEgqFgvdz7Bcc+LznCI+KlD1XEfUuWFWjS24MJ7sx3k=";
  };
in
  stdenv.mkDerivation {
    pname = "compass-extension-runtime";
    version = "0.28.4";
    inherit src;

    strictDeps = true;
    nativeBuildInputs = [nodejs];

    postPatch = ''
      local postPatchHooks=()
      source ${npmHooks.npmConfigHook}/nix-support/setup-hook
      npmRoot=src/typescript/api npmDeps=${apiDeps} npmConfigHook
      npmRoot=src/typescript/extension-manager npmDeps=${extensionManagerDeps} npmConfigHook
    '';

    buildPhase = ''
      runHook preBuild
      # node_modules are already installed offline by npmConfigHook above.
      SKIP_NPM_INSTALL=1 bash scripts/build-extension-runtime.sh
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      install -Dm644 src/typescript/extension-manager/dist/runtime.js \
        "$out/share/compass/extension-runtime.js"
      runHook postInstall
    '';

    passthru = {inherit apiDeps extensionManagerDeps;};

    meta = {
      description = "Extension runtime bundle for the Compass (Rust) engine";
      license = lib.licenses.gpl3Only;
      platforms = lib.platforms.linux;
    };
  }
