{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

let
  # The locked nixpkgs Spike recipe only lists Linux; use the same source and
  # build/install checks on Darwin, with a local platform override.
  spike = if pkgs.stdenv.hostPlatform.isDarwin then
    pkgs.spike.overrideAttrs (old: {
      # RISC-V SoftFloat uses a dummy commonNaN that its conversion functions
      # never read. Clang diagnoses its address being passed uninitialized.
      postPatch = (old.postPatch or "") + ''
        substituteInPlace softfloat/softfloat.mk.in \
          --replace-fail 'softfloat_CFLAGS = ' \
            'softfloat_CFLAGS = -Wno-error=uninitialized-const-pointer '
      '';
      meta = old.meta // {
        platforms = old.meta.platforms ++ lib.platforms.darwin;
      };
    })
  else
    pkgs.spike;
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
    pkgs.cargo-nextest
    rv64Tools
    spike
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
