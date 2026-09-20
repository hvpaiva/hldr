{ craneLib }:

rec {
  commonArgs = {
    src = craneLib.cleanCargoSource ../.;
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
    }
  );
}
