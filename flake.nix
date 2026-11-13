{
  description = "nx-edit: a native NixOS editor for the Line 6 HX Stomp";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
      # eframe dlopens these at runtime; NixOS has no global library path for them.
      runtimeLibs = with pkgs; [
        libxkbcommon
        wayland
        libGL
        vulkan-loader
        libx11
        libxcursor
        libxrandr
        libxi
      ];
      desktopItem = pkgs.makeDesktopItem {
        name = "nx-edit";
        desktopName = "nx-edit";
        genericName = "HX Stomp editor";
        comment = "Edit the presets of a Line 6 HX Stomp";
        exec = "nx-edit";
        icon = "nx-edit";
        categories = [ "AudioVideo" "Audio" ];
      };
      nx-edit = pkgs.rustPlatform.buildRustPackage {
        pname = "nx-edit";
        version = "0.1.0";
        src = pkgs.lib.cleanSource ./.;
        cargoLock.lockFile = ./Cargo.lock;
        cargoBuildFlags = [ "-p" "nx-app" ];
        cargoTestFlags = [ "--workspace" ];
        nativeBuildInputs = [ pkgs.makeWrapper ];
        # Replaces the global pipewire-jack LD_LIBRARY_PATH from /etc/set-environment, as the dev
        # shell does; 7z unpacks the HX Edit installer for --import-hxedit.
        postInstall = ''
          wrapProgram $out/bin/nx-edit \
            --set LD_LIBRARY_PATH ${pkgs.lib.makeLibraryPath runtimeLibs} \
            --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.p7zip ]}
          install -Dm644 assets/nx-edit.svg $out/share/icons/hicolor/scalable/apps/nx-edit.svg
          install -Dm644 ${desktopItem}/share/applications/nx-edit.desktop $out/share/applications/nx-edit.desktop
        '';
        meta.mainProgram = "nx-edit";
      };
      win = pkgs.pkgsCross.mingwW64;
      nx-edit-windows = win.rustPlatform.buildRustPackage {
        pname = "nx-edit-windows";
        version = "0.1.0";
        src = pkgs.lib.cleanSource ./.;
        cargoLock.lockFile = ./Cargo.lock;
        cargoBuildFlags = [ "-p" "nx-app" ];
        buildInputs = [ win.windows.pthreads ];
        nativeBuildInputs = [ pkgs.zip ];
        doCheck = false;
        postInstall = ''
          mkdir -p nx-edit
          cp $out/bin/nx-edit.exe LICENSE nx-edit/
          zip -r $out/nx-edit-windows-x86_64.zip nx-edit
        '';
      };
    in {
      packages.${system} = {
        default = nx-edit;
        windows = nx-edit-windows;
      };
      apps.${system}.default = { type = "app"; program = "${nx-edit}/bin/nx-edit"; };
      devShells.${system} = {
        default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc clippy rustfmt rust-analyzer pkg-config p7zip ];
          shellHook = ''
            export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath runtimeLibs}
          '';
        };
        windows = win.mkShell {
          nativeBuildInputs = [ win.buildPackages.cargo win.buildPackages.rustc pkgs.wineWow64Packages.stable ];
          buildInputs = [ win.windows.pthreads ];
          CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";
          CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "${win.stdenv.cc.targetPrefix}cc";
          CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER = "wine";
        };
      };
    };
}
