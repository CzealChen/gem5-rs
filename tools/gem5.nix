{ lib, stdenv, scons, m4, zlib }:

stdenv.mkDerivation (finalAttrs: {
  pname = "gem5-riscv";
  version = "26.0.0.0";

  src = builtins.fetchGit {
    url = "https://github.com/gem5/gem5.git";
    ref = "refs/tags/v${finalAttrs.version}";
    rev = "fed0e07e930fecf1e73dce94dd44e69a5bf70c46";
    shallow = true;
  };

  nativeBuildInputs = [ scons m4 ];
  buildInputs = [ scons.python zlib ];
  NIX_CFLAGS_COMPILE = "-g0";
  enableParallelBuilding = true;

  postPatch = ''
    substituteInPlace build_opts/RISCV --replace-fail RUBY=y RUBY=n
    patchShebangs --build build_tools ext/Kconfiglib
  '';

  buildPhase = ''
    runHook preBuild
    scons --ignore-style --without-tcmalloc -j"$NIX_BUILD_CORES" build/RISCV/gem5.opt
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    install -Dm755 build/RISCV/gem5.opt "$out/bin/gem5"
    runHook postInstall
  '';

  meta = {
    description = "gem5 computer-system simulator with RISC-V support";
    homepage = "https://www.gem5.org/";
    license = lib.licenses.bsd3;
    platforms = lib.platforms.linux;
    mainProgram = "gem5";
  };
})
