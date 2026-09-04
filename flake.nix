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
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
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
            cmakeWithLibdir = pkgs.writeShellScript "cmake-fastpotify" ''
              if [[ "$1" == "--build" ]]; then
                exec ${pkgs.cmake}/bin/cmake "$@"
              else
                exec ${pkgs.cmake}/bin/cmake "$@" -DCMAKE_INSTALL_LIBDIR=lib
              fi
            '';
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

            # The lock file contains git dependencies. fetchCargoVendor includes
            # them in the fixed-output dependency tree, unlike cargoLock alone.
            cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
              pname = "fastpotify";
              version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
              src = self;
              hash = "sha256-wC3tq8xj9tLYmZkvnsoHgYaTAtnwmktL1lAifeK0ui8=";
            };

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
                  # libprojectM links OpenGL directly and its GL loader needs
                  # X11 headers while it is built.
                  libGL
                  libx11
                ]
              )
              ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.apple-sdk ];

            # projectm-sys expects CMake to install into lib/, while CMake
            # defaults to lib64/ on NixOS.
            env.CMAKE = "${cmakeWithLibdir}";

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

      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
    };
}
