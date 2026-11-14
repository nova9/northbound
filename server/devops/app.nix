{ pkgs, ... }:

let
  northbound-server = pkgs.rustPlatform.buildRustPackage {
    pname = "northbound-server";
    version = "0.1.0";

    src = pkgs.fetchFromGitHub {
      owner = "nova9";
      repo = "northbound";
      rev = "08dc65fd97fd26e6e619bfe12a35231eea158d78";
      hash = "sha256-LN+0VGA4XqiXBfi+cHhizK0QUeuw45mn8DRgatJYf0I=";
    };

    cargoHash = "sha256-qvBzobtaeQJdecQ8OEI5vj3TzAOE0mJVDh3gdDwp4To=";

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

  services.caddy = {
    enable = true;

    virtualHosts.":80".extraConfig = ''
      reverse_proxy 127.0.0.1:3000
    '';
  };

  networking.firewall.allowedTCPPorts = [
    80
    9555
  ];
}
