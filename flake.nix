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
              dbus
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
                dbus
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

      packages = forAllSystems (
        pkgs:
        let
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
                  dbus
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
              pname = "spotifast";
              version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
              src = self;

              # The lock file contains git dependencies. fetchCargoVendor includes
              # them in the fixed-output dependency tree, unlike cargoLock alone.
              cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
                pname = "fastpotify";
                version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
                src = self;
                hash = "sha256-J2GEGImCj/Og+22EayPygtxp/qwOTYMzMriydHPHaHE=";
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
              # The command-alias regression starts its own bus, isolated from
              # the listener's desktop and running application.
              nativeCheckInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.dbus ];
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

            # Nix's Rust check hook deliberately points SSL_CERT_FILE at a
            # missing path. The librespot proxy regression test constructs a
            # TLS connector before asserting its CONNECT request, so give the
            # test an explicit, sandboxed trust store.
            preCheck = ''
              export SSL_CERT_FILE="${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
            '';

              # The GUI dlopens its Wayland, X11 and GL libraries at run time.
              postFixup = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
                wrapProgram $out/bin/fastpotify \
                  --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibs}
                wrapProgram $out/bin/spotifast \
                  --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibs}
              '';

              postInstall = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
                install -Dm644 packaging/applications/spotifast.desktop \
                  $out/share/applications/spotifast.desktop
                install -Dm644 packaging/icons/spotifast.svg \
                  $out/share/icons/hicolor/scalable/apps/spotifast.svg
                install -Dm644 contrib/omarchy/spotifast.json.tpl \
                  $out/share/spotifast/omarchy/spotifast.json.tpl
                install -Dm755 contrib/omarchy/spotifast-theme \
                  $out/share/spotifast/omarchy/spotifast-theme
              '';

              meta = {
                description = "Fast native Spotify client with local playback and Spotify Connect";
                homepage = "https://spotifast.rocks";
                license = pkgs.lib.licenses.mit;
                mainProgram = "spotifast";
              };
            };

          fastpotify-app =
            let
              version = pkgs.lib.getVersion fastpotify;
              build = pkgs.lib.head (pkgs.lib.splitString "-" version);
              icon =
                pkgs.runCommand "fastpotify-icon"
                  {
                    nativeBuildInputs = [ pkgs.icnsify ];
                  }
                  ''
                    icnsify ${./packaging/macos/icon-1024.png} -o $out
                  '';
            in
            pkgs.runCommand "spotifast-app"
              {
                meta = {
                  description = "A native Spotify client in a macOS app bundle";
                  homepage = "https://spotifast.rocks";
                  license = pkgs.lib.licenses.mit;
                  platforms = pkgs.lib.platforms.darwin;
                };
              }
              ''
                app="$out/Applications/Spotifast.app/Contents"
                mkdir -p "$app/MacOS" "$app/Resources"
                cp ${fastpotify}/bin/fastpotify "$app/MacOS/fastpotify"
                chmod 755 "$app/MacOS/fastpotify"
                cp ${icon} "$app/Resources/fastpotify.icns"
                sed -e "s/__VERSION__/${version}/g" -e "s/__BUILD__/${build}/g" \
                  ${./packaging/macos/Info.plist} > "$app/Info.plist"
                /usr/bin/codesign --force --sign - \
                  "$out/Applications/Spotifast.app"
                /usr/bin/codesign --verify --strict \
                  "$out/Applications/Spotifast.app"
              '';
        in
        {
          default = fastpotify;
          inherit fastpotify;
          spotifast = fastpotify;
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isDarwin {
          inherit fastpotify-app;
          spotifast-app = fastpotify-app;
        }
      );

      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
    };
}
