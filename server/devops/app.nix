# NixOS module for the production Northbound server on AWS EC2.
# Deploys three components on one host:
#   1. truenorthd  — unpruned TrueNorth mainnet full node (loopback RPC only)
#   2. northbound  — wallet & solo-mining HTTP API + SQLite indexer (127.0.0.1:3000)
#   3. caddy       — HTTPS reverse proxy terminating TLS for public traffic
{ pkgs, lib, ... }:

let
  # Update both values together when deploying a new Northbound release.
  releaseTag = "v0.2.0-alpha.4";
  releaseSha256 = "sha256-pvmx8hvf3gl4TnhGjpD+zRVYVKMIpfxE2uk2bn8kne8=";

  # Prebuilt Northbound server binary from GitHub Releases.
  # autoPatchelfHook rewrites its dynamic linker and shared library paths for NixOS.
  northbound-server = pkgs.stdenv.mkDerivation {
    name = "northbound-server";
    src = pkgs.fetchurl {
      url = "https://github.com/nova9/northbound/releases/download/${releaseTag}/northbound-server-linux-x64";
      hash = releaseSha256;
    };
    dontUnpack = true;
    nativeBuildInputs = [ pkgs.autoPatchelfHook ];
    buildInputs = [
      pkgs.stdenv.cc.cc.lib
      pkgs.openssl
    ];

    installPhase = ''
      install -Dm755 "$src" "$out/bin/server"
    '';
  };

  # Prebuilt TrueNorth Core release binaries (daemon, CLI, and miner).
  truenorth = pkgs.stdenv.mkDerivation {
    pname = "truenorth";
    version = "30.5.5";

    src = pkgs.fetchurl {
      url = "https://github.com/truenorth-project/truenorth/releases/download/v30.5.5/truenorth-v30.5.5-linux-x86_64.tar.gz";
      hash = "sha256-ROb1F2X74s5jn2zjAwknQNu/0VY6gFlFGY/qYgZIgrg=";
    };

    nativeBuildInputs = [ pkgs.autoPatchelfHook ];

    # Shared libraries linked by the prebuilt TrueNorth Core binaries.
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
  # 4 GiB swap file as memory headroom during initial block download and indexing.
  swapDevices = [
    {
      device = "/var/lib/swapfile";
      size = 4096; # MiB
    }
  ];

  # Expose binaries on PATH for administrative commands (e.g. truenorth-cli, nixfmt).
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

  # Separate unprivileged system accounts for the node and API server.
  users.groups.truenorth = { };
  users.groups.northbound = { };

  users.users.truenorth = {
    isSystemUser = true;
    group = "truenorth";
  };

  users.users.northbound = {
    isSystemUser = true;
    group = "northbound";
    # Group membership grants read-only access to /var/lib/truenorth/.cookie.
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

      # Key flags:
      # -prune=0: the Northbound indexer requires full historical blocks.
      # -rpcbind / -rpcallowip: keep daemon RPC strictly on loopback.
      # -rpccookieperms=group: allow the northbound user to authenticate via cookie.
      # -txindex=1: required for /v1/transactions/status fallback lookups.
      ExecStart = "${truenorth}/bin/truenorthd -chain=main -datadir=/var/lib/truenorth -printtoconsole -server=1 -prune=0 -rpcbind=127.0.0.1 -rpcallowip=127.0.0.1 -rpccookiefile=/var/lib/truenorth/.cookie -rpccookieperms=group -blockfilterindex=1 -txindex=1";

      Restart = "on-failure";
      RestartSec = 10;

      # Creates /var/lib/truenorth owned by truenorth:truenorth; 0710 lets the
      # northbound user traverse into the directory to read the RPC cookie.
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
      TRUENORTH_MINING_ENABLED = "true";
    };

    serviceConfig = {
      User = "northbound";
      Group = "northbound";

      ExecStart = "${northbound-server}/bin/server";
      # Private /var/lib/northbound directory for the SQLite index database.
      StateDirectory = "northbound";
      StateDirectoryMode = "0700";
      # Retry automatically while truenorthd finishes starting up or creating its cookie.
      Restart = "always";
      RestartSec = 5;
    };
  };

  # Enables shell access via AWS Systems Manager (Session Manager) without opening SSH.
  services.amazon-ssm-agent.enable = true;

  # Public HTTPS reverse proxy with sliding-window rate limiting.
  services.caddy = {
    enable = true;

    # Stock Caddy does not bundle rate limiting; build with the caddy-ratelimit plugin.
    # Run `nixos-rebuild switch` once and replace `lib.fakeHash` with the hash Nix reports.
    package = pkgs.caddy.withPlugins {
      plugins = [ "github.com/mholt/caddy-ratelimit@v0.1.0" ];
      hash = lib.fakeHash;
    };

    globalConfig = ''
      order rate_limit before reverse_proxy
    '';

    virtualHosts."api.northboundwallet.org".extraConfig = ''
      rate_limit {
        # Apply to all API routes except /ready health probes.
        zone api_per_ip {
          match {
            not path /ready
          }
          key {remote_host}
          events 30
          window 1s
        }

        zone api_global {
          match {
            not path /ready
          }
          key static
          events 300
          window 1s
        }
      }

      handle_errors {
        @429 expression {err.status_code} == 429
        handle @429 {
          header Content-Type application/json
          header Cache-Control no-store
          respond `{"message":"Too many requests; retry shortly."}` 429
        }
      }

      reverse_proxy 127.0.0.1:3000
    '';
  };

  # Public ports only — daemon RPC (9554) and Northbound HTTP (3000) stay loopback-only.
  networking.firewall.allowedTCPPorts = [
    80 # HTTP (ACME challenges and HTTPS redirects)
    443 # HTTPS (Caddy -> Northbound API)
    9555 # TrueNorth P2P network
  ];
}
