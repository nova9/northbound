{ pkgs, ... }:

let
  northbound-server = pkgs.rustPlatform.buildRustPackage {
    pname = "northbound-server";
    version = "0.1.0";

    src = pkgs.fetchFromGitHub {
      owner = "nova9";
      repo = "northbound";
      rev = "66fa1d91444185a60c8273a7b8dc20d2d3ee130b";
      hash = "sha256-2rUJSk4inRR3M/DWSyTiKT1JmSJ5DoUx6Mw2qWU5+LY=";
    };

    cargoHash = "sha256-ywn7hFJNUGp4tLaEDHPjnXHEYc5MZU0uuDRAFZmPfN4=";

    cargoBuildFlags = [
      "-p"
      "server"
    ];
    cargoTestFlags = [
      "-p"
      "server"
    ];
    cargoInstallFlags = [
      "-p"
      "server"
    ];

    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = [ pkgs.openssl ];

    # Reqwest initializes its TLS verifier even for local HTTP tests.
    SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";

    CARGO_BUILD_JOBS = "1";
    CARGO_PROFILE_RELEASE_OPT_LEVEL = "0";
    CARGO_PROFILE_RELEASE_LTO = "false";
    CARGO_PROFILE_RELEASE_DEBUG = "0";
  };

  truenorth = pkgs.stdenv.mkDerivation {
    pname = "truenorth";
    version = "30.5.4";

    src = pkgs.fetchurl {
      url = "https://github.com/truenorth-project/truenorth/releases/download/v30.5.4/truenorth-v30.5.4-linux-x86_64.tar.gz";
      hash = "sha256-uJ5lY7+TuYU9JNrojzksT+CBWYsc+ehXn11nrEXCKgU=";
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

      ExecStart = "${truenorth}/bin/truenorthd -datadir=/var/lib/truenorth -printtoconsole -server=1 -rpcbind=127.0.0.1 -rpcallowip=127.0.0.1 -rpccookiefile=/var/lib/truenorth/.cookie -rpccookieperms=group -blockfilterindex=1 -txindex=1";

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
      TRUENORTH_SERVER_REQUESTS_PER_SECOND = "500";
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
