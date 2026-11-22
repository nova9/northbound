{ pkgs, ... }:

let
  releaseTag = "v0.2.0-alpha.4";
  releaseSha256 = "sha256-pvmx8hvf3gl4TnhGjpD+zRVYVKMIpfxE2uk2bn8kne8=";

  northbound-server = pkgs.stdenv.mkDerivation {
    name = "northbound-server";
    src = pkgs.fetchurl {
      url = "https://github.com/nova9/northbound/releases/download/${releaseTag}/northbound-server-linux-x64";
      hash = releaseSha256;
    };
    dontUnpack = true;
    nativeBuildInputs = [ pkgs.autoPatchelfHook ];
    buildInputs = [ pkgs.stdenv.cc.cc.lib pkgs.openssl ];

    installPhase = ''
      install -Dm755 "$src" "$out/bin/server"
    '';
  };

  truenorth = pkgs.stdenv.mkDerivation {
    pname = "truenorth";
    version = "30.5.5";

    src = pkgs.fetchurl {
      url = "https://github.com/truenorth-project/truenorth/releases/download/v30.5.5/truenorth-v30.5.5-linux-x86_64.tar.gz";
      hash = "sha256-ROb1F2X74s5jn2zjAwknQNu/0VY6gFlFGY/qYgZIgrg=";
    };

    nativeBuildInputs = [ pkgs.autoPatchelfHook ];

    buildInputs = with pkgs; [
      stdenv.cc.cc.lib
      openssl
      libevent
      sqlite
      boost
      zlib
      numactl
    ];

    dontUnpack = true;

    installPhase = ''
      runHook preInstall

      mkdir -p $out/bin unpack
      tar -xzf "$src" -C unpack

      for binary in truenorthd truenorth-cli truenorth-miner; do
        binaryPath=$(find unpack -type f -name "$binary" -print -quit)
        test -n "$binaryPath"
        install -m755 "$binaryPath" "$out/bin/$binary"
      done

      runHook postInstall
    '';
  };

in
{
  swapDevices = [
    {
      device = "/var/lib/swapfile";
      size = 4096; # MiB
    }
  ];

  environment.systemPackages = [
    northbound-server
    truenorth
    pkgs.nixfmt
  ];

  programs.nano = {
    enable = true;
    nanorc = ''
      set linenumbers
      set mouse
      set tabsize 4
      set tabstospaces
      set autoindent
      set softwrap
    '';
  };

  users.groups.truenorth = { };
  users.groups.northbound = { };

  users.users.truenorth = {
    isSystemUser = true;
    group = "truenorth";
  };

  users.users.northbound = {
    isSystemUser = true;
    group = "northbound";
    extraGroups = [ "truenorth" ];
  };

  systemd.services.truenorth = {
    description = "TrueNorth Full Node";
    wantedBy = [ "multi-user.target" ];
    after = [ "network-online.target" ];
    wants = [ "network-online.target" ];

    serviceConfig = {
      User = "truenorth";
      Group = "truenorth";

      ExecStart = "${truenorth}/bin/truenorthd -chain=main -datadir=/var/lib/truenorth -printtoconsole -server=1 -prune=0 -rpcbind=127.0.0.1 -rpcallowip=127.0.0.1 -rpccookiefile=/var/lib/truenorth/.cookie -rpccookieperms=group -blockfilterindex=1 -txindex=1";

      Restart = "on-failure";
      RestartSec = 10;

      StateDirectory = "truenorth";
      StateDirectoryMode = "0710";
    };
  };

  systemd.services.northbound = {
    description = "Northbound Server";
    wantedBy = [ "multi-user.target" ];
    after = [ "truenorth.service" ];
    wants = [ "truenorth.service" ];

    environment = {
      TRUENORTH_NETWORK = "main";
      TRUENORTH_RPC_URL = "http://127.0.0.1:9554";
      TRUENORTH_RPC_COOKIE_PATH = "/var/lib/truenorth/.cookie";
      TRUENORTH_INDEX_DATABASE = "/var/lib/northbound/chain.sqlite";
      TRUENORTH_SERVER_REQUESTS_PER_SECOND = "300";
      TRUENORTH_MINING_ENABLED = "true";
      TRUENORTH_MINING_REQUESTS_PER_SECOND = "100";
    };

    serviceConfig = {
      User = "northbound";
      Group = "northbound";

      ExecStart = "${northbound-server}/bin/server";
      StateDirectory = "northbound";
      StateDirectoryMode = "0700";
      Restart = "always";
      RestartSec = 5;
    };
  };

  services.amazon-ssm-agent.enable = true;

  services.caddy = {
    enable = true;
    virtualHosts."api.northboundwallet.org".extraConfig = ''
      reverse_proxy 127.0.0.1:3000
    '';

    # Keep the IP endpoint available for existing wallet releases.
    # IP clients omit SNI; EC2 exposes a private address to Caddy locally.
    globalConfig = ''
      default_sni 13.55.12.147
    '';

    virtualHosts."https://13.55.12.147".extraConfig = ''
      tls {
        issuer acme {
          dir https://acme-v02.api.letsencrypt.org/directory
          profile shortlived
        }
      }

      reverse_proxy 127.0.0.1:3000
    '';
  };

  networking.firewall.allowedTCPPorts = [
    80
    443
    9555
  ];
}
