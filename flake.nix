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
        xorg.libX11
        xorg.libXcursor
        xorg.libXrandr
        xorg.libXi
      ];
    in {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [ cargo rustc clippy rustfmt rust-analyzer pkg-config p7zip ];
        # Replaces the global pipewire-jack LD_LIBRARY_PATH from /etc/set-environment.
        shellHook = ''
          export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath runtimeLibs}
        '';
      };
    };
}
