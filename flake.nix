{
  description = "HTTP index provider";
  inputs.flake-utils.url = "github:numtide/flake-utils";
  inputs.rust-overlay.url = "github:oxalica/rust-overlay";
  inputs.nixpkgs.url = "github:nixos/nixpkgs/release-21.11";

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        rustStable = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" ];
        };
      in
      {
        devShell = pkgs.mkShell {
          buildInputs = [
            pkgs.go
            rustStable
          ] ++ (if (system == "aarch64-darwin" || system == "x86_64-darwin") then [
            pkgs.darwin.apple_sdk.frameworks.Security
          ] else [ ]);
        };
      });
}
