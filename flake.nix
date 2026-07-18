{
  description = "Fast, keyboard-driven terminal UI for todo.txt";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    inputs@{ self, flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];
      perSystem =
        {
          pkgs,
          ...
        }:
        {
          packages.default = pkgs.callPackage ./package.nix { };
        };
      flake = {
        overlays.default = final: _prev: {
          tuxedo = self.packages.${final.stdenv.hostPlatform.system}.default;
        };
      };
    };
}
