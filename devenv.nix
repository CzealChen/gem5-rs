{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

let
  rv64 = pkgs.pkgsCross.riscv64-embedded.stdenv.cc;
  # Expose the prefixed GCC and binutils alongside the native Rust linker.
  rv64Tools = pkgs.runCommand "rv64-elf-tools" { } ''
    mkdir -p "$out"
    ln -s ${rv64}/bin "$out/bin"
  '';
in
{
  # https://devenv.sh/basics/
  env.GREET = "devenv";

  # https://devenv.sh/packages/
  # Tool versions come from the nixpkgs revision in devenv.lock.
  packages = [
    pkgs.git
    pkgs.just
    rv64Tools
    pkgs.spike
    pkgs.dtc # Spike compiles its device tree at startup.
  ];

  # https://devenv.sh/languages/
  languages.python = {
    enable = true;
    package = pkgs.python313;
    venv.enable = true;
    uv = {
      enable = true;
      sync.enable = true;
      sync.arguments = [ "--locked" ];
    };
  };

  languages.rust = {
    enable = true;
    channel = "stable";
  };

  profiles.gem5.module.packages = [
    (let
      gem5 = pkgs.callPackage ./tools/gem5.nix { };
    in
    pkgs.writeShellScriptBin "gem5" ''
      export LD_LIBRARY_PATH="${lib.makeLibraryPath [ pkgs.stdenv.cc.cc ]}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
      exec ${lib.getExe gem5} "$@"
    '')
  ];

  # https://devenv.sh/processes/
  # processes.dev.exec = "${lib.getExe pkgs.watchexec} -n -- ls -la";

  # https://devenv.sh/services/
  # services.postgres.enable = true;

  # https://devenv.sh/scripts/
  scripts.hello.exec = ''
    echo hello from $GREET
  '';

  # https://devenv.sh/basics/
  enterShell = ''
    hello         # Run scripts directly
    git --version # Use packages
  '';

  # https://devenv.sh/tasks/
  # tasks = {
  #   "myproj:setup".exec = "mytool build";
  #   "devenv:enterShell".after = [ "myproj:setup" ];
  # };

  # https://devenv.sh/tests/
  enterTest = ''
    echo "Running tests"
    git --version | grep --color=auto "${pkgs.git.version}"
  '';

  # https://devenv.sh/git-hooks/
  # git-hooks.hooks.shellcheck.enable = true;

  # See full reference at https://devenv.sh/reference/options/
}
