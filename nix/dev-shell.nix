{ pkgs }:
let
  toolchainSpec = (builtins.fromTOML (builtins.readFile ../rust-toolchain.toml)).toolchain;
  toolchain = (pkgs.rust-bin.fromRustupToolchainFile ../rust-toolchain.toml).override {
    extensions = toolchainSpec.components ++ [ "rust-src" "rust-analyzer" ];
  };

  # Loaded at run time by the window, GPU and sound code.
  runtimeLibraries = with pkgs; [
    vulkan-loader
    libGL
    libxkbcommon
    wayland
    libx11
    libxcb
    libxcursor
    libxi
    libxrandr
    fontconfig
    freetype
    alsa-lib
  ];
in pkgs.mkShell {
  name = "dutch-flashcards-dev";

  packages = with pkgs; [
    toolchain
    gcc
    pkg-config
    piper-tts # Speaks the Dutch on each card.
    whisper-cpp-vulkan # Hears the learner read a card aloud (runs on the GPU).
    weston # Headless compositor for window checks and screenshots.
  ];

  buildInputs = runtimeLibraries;
  RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";

  shellHook = ''
    export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath runtimeLibraries}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  '';
}
