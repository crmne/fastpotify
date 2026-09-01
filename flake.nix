{
  description = "Spotify, native and fast";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # rust-toolchain.toml pins the compiler so local builds and CI agree.
    # This reads that file rather than restating the version here.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      ...
    }:
    let
      # x86_64-darwin is gone: nixpkgs 26.11 dropped it, so its
      # derivations no longer evaluate. Override `systems` if you need it.
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ (import rust-overlay) ];
            }
          )
        );
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages =
            with pkgs;
            [
              (rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
              rust-analyzer
              pkg-config
              # libprojectM (MilkDrop) is built from source by CMake, and its
              # bindings by bindgen, which needs libclang.
              cmake
              rustPlatform.bindgenHook
            ]
            ++ lib.optionals stdenv.hostPlatform.isDarwin [
              apple-sdk
            ]
            ++ lib.optionals stdenv.hostPlatform.isLinux [
              alsa-lib
              libpulseaudio
              libxkbcommon
              wayland
              libGL
              libx11
              libxcursor
              libxi
              libxrandr
            ];
          # The GUI dlopens its Wayland, X11 and GL libraries at run time.
          LD_LIBRARY_PATH = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux (
            pkgs.lib.makeLibraryPath (
              with pkgs;
              [
                libxkbcommon
                wayland
                libGL
                libx11
                libxcursor
                libxi
                libxrandr
              ]
            )
          );
        };
      });

      packages = forAllSystems (pkgs: rec {
        default = fastpotify;
        fastpotify =
          let
            toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
            rustPlatform = pkgs.makeRustPlatform {
              cargo = toolchain;
              rustc = toolchain;
            };
            runtimeLibs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
              with pkgs;
              [
                libxkbcommon
                wayland
                libGL
                libx11
                libxcursor
                libxi
                libxrandr
              ]
            );
          in
          rustPlatform.buildRustPackage {
            pname = "fastpotify";
            version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            # Git dependencies in Cargo.lock need a fixed-output hash per
            # crate so the vendor directory is reproducible.
            cargoLock.outputHashes = {
              "librespot-audio-0.8.0" = "sha256-TkHdN/dugdmK5iWmcvxGhz+0Cynki4/nNpp85F/qF/0=";
              "librespot-connect-0.8.0" = "sha256-TkHdN/dugdmK5iWmcvxGhz+0Cynki4/nNpp85F/qF/0=";
              "librespot-core-0.8.0" = "sha256-TkHdN/dugdmK5iWmcvxGhz+0Cynki4/nNpp85F/qF/0=";
              "librespot-metadata-0.8.0" = "sha256-TkHdN/dugdmK5iWmcvxGhz+0Cynki4/nNpp85F/qF/0=";
              "librespot-oauth-0.8.0" = "sha256-TkHdN/dugdmK5iWmcvxGhz+0Cynki4/nNpp85F/qF/0=";
              "librespot-playback-0.8.0" = "sha256-TkHdN/dugdmK5iWmcvxGhz+0Cynki4/nNpp85F/qF/0=";
              "librespot-protocol-0.8.0" = "sha256-TkHdN/dugdmK5iWmcvxGhz+0Cynki4/nNpp85F/qF/0=";
              "projectm-sys-1.2.3" = "sha256-sgI6IOCpQUvdc5acQ1wjCM5mhfz2EPZmoeuyNLGB5UI=";
            };

            # projectm-sys' build.rs links the static library from
            # $OUT_DIR/lib, but CMake's GNUInstallDirs installs it into
            # lib64 on x86_64 Linux, so the link step cannot find it.
            # Force `lib` in the vendored CMakeLists before the build
            # script runs. (Vendored git crates carry no file checksums.)
            preBuild = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              substituteInPlace "$NIX_BUILD_TOP/cargo-vendor-dir/projectm-sys-1.2.3/libprojectM/CMakeLists.txt" \
                --replace 'set(PROJECTM_LIB_DIR "''${CMAKE_INSTALL_LIBDIR}" CACHE' 'set(PROJECTM_LIB_DIR "lib" CACHE'
            '';

            nativeBuildInputs =
              with pkgs;
              [
                pkg-config
                # libprojectM (MilkDrop) is built from source by CMake, and
                # its bindings by bindgen, which needs libclang.
                cmake
                rustPlatform.bindgenHook
              ]
              ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ makeWrapper ];
            buildInputs =
              pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
                with pkgs;
                [
                  alsa-lib
                  libpulseaudio
                  # libprojectM links OpenGL directly.
                  libGL
                  # Its vendored glad GLX loader compiles against X11 headers.
                  libx11
                ]
              )
              ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.apple-sdk ];

            # The GUI dlopens its Wayland, X11 and GL libraries at run time.
            postFixup = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              wrapProgram $out/bin/fastpotify \
                --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibs}
            '';

            postInstall = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              install -Dm644 packaging/applications/fastpotify.desktop \
                $out/share/applications/fastpotify.desktop
              install -Dm644 packaging/icons/fastpotify.svg \
                $out/share/icons/hicolor/scalable/apps/fastpotify.svg
            '';

            meta = {
              description = "Fast native Spotify client with local playback and Spotify Connect";
              homepage = "https://fastpotify.rocks";
              license = pkgs.lib.licenses.mit;
              mainProgram = "fastpotify";
            };
          };
      });

      nixosModules.default =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.programs.fastpotify;
        in
        {
          options.programs.fastpotify = {
            enable = lib.mkEnableOption "fastpotify";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.system}.fastpotify;
              defaultText = lib.literalExpression "fastpotify.packages.\${pkgs.system}.fastpotify";
              description = "The fastpotify package to install.";
            };
          };

          config = lib.mkIf cfg.enable {
            environment.systemPackages = [ cfg.package ];
          };
        };

      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
    };
}
