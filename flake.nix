{
  description = "lrat, a fast, keyboard-first terminal client for Linear";

  inputs = {
    # nixos-26.05 as of 2026-11-01. Bump to a commit at least two weeks old,
    # then run `nix flake lock` and commit flake.lock.
    nixpkgs.url = "github:NixOS/nixpkgs/64aec40e18595d342f25f120069f2cb63c3947b4";
    crane.url = "github:ipetkov/crane/v0.24.0";
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
    }:
    let
      # The platforms the release ships for.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      lratFor =
        pkgs:
        let
          craneLib = crane.mkLib pkgs;
          commonArgs = {
            # The GraphQL documents and built-in themes are compiled in with include_str!.
            src = pkgs.lib.cleanSourceWith {
              src = ./.;
              filter =
                path: type:
                craneLib.filterCargoSources path type || builtins.match ".*\\.(graphql|json)$" path != null;
            };
            strictDeps = true;
            # aws-lc-sys may build through CMake; it is not the project's build system.
            nativeBuildInputs = [ pkgs.cmake ];
            dontUseCmakeConfigure = true;
            # CI runs the test suite with insta and the fake server; this build
            # checks that lrat compiles and links.
            doCheck = false;
          };
        in
        craneLib.buildPackage (
          commonArgs
          // {
            cargoArtifacts = craneLib.buildDepsOnly commonArgs;
            meta = {
              description = "A fast, keyboard-first terminal client for Linear";
              homepage = "https://github.com/cxrlos/linear-rat";
              license = with pkgs.lib.licenses; [
                mit
                asl20
              ];
              mainProgram = "lrat";
            };
          }
        );
    in
    {
      packages = forAllSystems (pkgs: {
        default = lratFor pkgs;
      });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/lrat";
          meta.description = "Run lrat";
        };
      });

      checks = forAllSystems (pkgs: {
        lrat = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          # rustup, as in CI, so `cargo +1.88 check` reaches the MSRV
          # toolchain next to stable.
          packages = [
            pkgs.rustup
            pkgs.cargo-deny
            pkgs.cargo-insta
            pkgs.cmake
          ];
          shellHook = ''
            rustup toolchain list | grep -q '^stable' \
              || rustup toolchain install stable --profile minimal --component rustfmt,clippy
            rustup toolchain list | grep -q '^1\.88' \
              || rustup toolchain install 1.88 --profile minimal
            rustup show active-toolchain >/dev/null 2>&1 || rustup default stable
          '';
        };
      });
    };
}
