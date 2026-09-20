{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.hldr;
in
{
  options.services.hldr = {
    enable = lib.mkEnableOption "hldr-server";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.hldr;
      description = "hldr package providing hldr-server.";
    };

    listenAddress = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1:8080";
      description = "Bind address for the public listener.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.hldr = {
      description = "hldr-server";
      after = [ "network.target" ];
      wantedBy = [ "multi-user.target" ];
      environment = {
        HLDR_ADDR = cfg.listenAddress;
        HLDR_LOG = "info";
      };
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/hldr-server";
        DynamicUser = true;
        StateDirectory = "hldr";
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        NoNewPrivileges = true;
        Restart = "always";
        RestartSec = "2s";
      };
    };
  };
}
