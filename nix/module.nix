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

    contentDir = lib.mkOption {
      type = lib.types.path;
      default = "${cfg.package}/share/hldr/content";
      description = "Directory of desired-state markdown and YAML.";
    };

    origin = lib.mkOption {
      type = lib.types.str;
      default = "https://hvpaiva.dev";
      description = "Canonical public origin for sitemap, robots, and Open Graph URLs.";
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
        HLDR_CONTENT_DIR = "${cfg.contentDir}";
        HLDR_ORIGIN = cfg.origin;
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
