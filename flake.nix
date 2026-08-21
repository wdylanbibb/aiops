{
  description = "AIOps Kubernetes incident collection and diagnostics";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in {
      packages = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          aiops = pkgs.rustPlatform.buildRustPackage {
            pname = "aiops";
            version = "0.1.0";
            src = nixpkgs.lib.fileset.toSource {
              root = ./.;
              fileset = nixpkgs.lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./crates
              ];
            };
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [ "-p" "aiops-cli" ];
            cargoTestFlags = [ "--workspace" ];
          };

          test = pkgs.writeShellApplication {
            name = "aiops-test";
            runtimeInputs = [ pkgs.cargo pkgs.clippy pkgs.rustc pkgs.rustfmt ];
            text = ''
              cargo fmt --all -- --check
              cargo clippy --workspace --all-targets -- -D warnings
              cargo test --workspace
            '';
          };

          deps = pkgs.writeShellApplication {
            name = "aiops-deps";
            runtimeInputs = [ pkgs.cargo ];
            text = ''
              cargo fetch --locked "$@"
            '';
          };

          test-kind = pkgs.writeShellApplication {
            name = "aiops-test-kind";
            runtimeInputs = [
              pkgs.bash
              pkgs.cargo
              pkgs.coreutils
              pkgs.gnugrep
              pkgs.jq
              pkgs.kind
              pkgs.kubectl
              pkgs.rustc
            ];
            text = ''
              exec bash tests/e2e/run.sh "$@"
            '';
          };

          run = pkgs.writeShellApplication {
            name = "aiops-run";
            text = ''
              exec ${aiops}/bin/aiops "$@"
            '';
          };
        in {
          default = aiops;
          inherit aiops deps run test test-kind;
        });

      apps = forAllSystems (system:
        let
          packages = self.packages.${system};
          app = program: {
            type = "app";
            program = "${program}/bin/${program.name}";
          };
        in {
          default = app packages.run;
          run = app packages.run;
          test = app packages.test;
          test-kind = app packages.test-kind;
          deps = app packages.deps;
        });

      checks = forAllSystems (system: {
        inherit (self.packages.${system}) aiops;
      });

      devShells = forAllSystems (system:
        let pkgs = nixpkgs.legacyPackages.${system};
        in {
          default = pkgs.mkShell {
            packages = [
              pkgs.cargo
              pkgs.clippy
              pkgs.jq
              pkgs.kind
              pkgs.kubectl
              pkgs.rustc
              pkgs.rustfmt
            ];
          };
        });
    };
}
