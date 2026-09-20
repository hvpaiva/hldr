{ craneLib, pkgs }:

let
  inherit (pkgs) lib;
  src = lib.cleanSourceWith {
    src = ../.;
    filter =
      path: type:
      let
        p = toString path;
      in
      (craneLib.filterCargoSources path type)
      || lib.hasInfix "/migrations/" p
      || lib.hasSuffix "/migrations" p
      || lib.hasInfix "/content/" p
      || lib.hasSuffix "/content" p;
  };
in
rec {
  commonArgs = {
    inherit src;
    strictDeps = true;
    pname = "hldr";
    version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).workspace.package.version;
    cargoExtraArgs = "--workspace";
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;

  hldr = craneLib.buildPackage (
    commonArgs
    // {
      inherit cargoArtifacts;
      doCheck = false;
      postInstall = ''
        mkdir -p $out/share/hldr
        cp -R content $out/share/hldr/content
      '';
    }
  );
}
