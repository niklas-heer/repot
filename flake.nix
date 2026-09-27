{
  description = "Safe Git checkout management, built from source";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      systems = [ "aarch64-darwin" "x86_64-darwin" "aarch64-linux" "x86_64-linux" ];
      eachSystem = nixpkgs.lib.genAttrs systems;
      forSystem = system:
        let
          pkgs = import nixpkgs { inherit system; overlays = [ rust-overlay.overlays.default ]; };
          manifest = builtins.fromTOML (builtins.readFile ./Cargo.toml);
          channel = (builtins.fromTOML (builtins.readFile ./rust-toolchain.toml)).toolchain.channel;
          toolchain = pkgs.rust-bin.stable.${channel}.minimal;
          platform = pkgs.makeRustPlatform { cargo = toolchain; rustc = toolchain; };
          shells = with pkgs; [ bash zsh fish nushell ];
          package = platform.buildRustPackage {
            pname = "repot";
            version = manifest.package.version;
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions [
                ./Cargo.toml ./Cargo.lock ./rust-toolchain.toml ./clippy.toml
                ./src ./vendor ./tests ./examples ./assets ./docs ./scripts ./README.md ./LICENSE
              ];
            };
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.makeWrapper ];
            nativeCheckInputs = [ pkgs.gitMinimal pkgs.unixtools.ps ] ++ shells;
            doCheck = true;
            preCheck = ''
              export HOME="$TMPDIR/repot-build-home"
              mkdir -p "$HOME"
            '';
            postInstall = ''
              wrapProgram "$out/bin/repot" --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.gitMinimal ]}
            '';
            doInstallCheck = true;
            installCheckPhase = ''
              runHook preInstallCheck
              "$out/bin/repot" --version
              runHook postInstallCheck
            '';
            meta = {
              description = manifest.package.description;
              homepage = manifest.package.repository;
              license = pkgs.lib.licenses.mit;
              mainProgram = "repot";
              platforms = systems;
            };
          };
        in { inherit pkgs package toolchain shells; };
    in {
      packages = eachSystem (system: { default = (forSystem system).package; });
      apps = eachSystem (system: {
        default = { type = "app"; program = "${self.packages.${system}.default}/bin/repot"; };
      });
      checks = eachSystem (system: { package = self.packages.${system}.default; });
      devShells = eachSystem (system:
        let env = forSystem system;
        in { default = env.pkgs.mkShell { packages = [ env.toolchain env.pkgs.gitMinimal ] ++ env.shells; }; });
    };
}
