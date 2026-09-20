{
  description = "hldr — hvpaiva.dev server and CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
    }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      craneLib = crane.mkLib pkgs;
      crate = import ./nix/crate.nix { inherit craneLib; };
    in
    {
      overlays.default = _final: prev: {
        hldr = (import ./nix/crate.nix { craneLib = crane.mkLib prev; }).hldr;
      };

      packages.${system} = {
        default = crate.hldr;
        inherit (crate) hldr;
      };

      nixosModules.hldr = ./nix/module.nix;

      checks.${system} = {
        inherit (crate) hldr;

        clippy = craneLib.cargoClippy (
          crate.commonArgs
          // {
            inherit (crate) cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          }
        );

        fmt = craneLib.cargoFmt { inherit (crate.commonArgs) src; };

        deny = craneLib.cargoDeny { inherit (crate.commonArgs) src; };

        test = craneLib.cargoTest (crate.commonArgs // { inherit (crate) cargoArtifacts; });
      };

      devShells.${system}.default = craneLib.devShell {
        packages = [
          pkgs.cargo-deny
        ];
      };

      formatter.${system} = pkgs.nixfmt-rfc-style;
    };
}
