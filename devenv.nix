{ pkgs, ... }:
{
  languages.rust = {
    enable = true;
    toolchainFile = ./rust-toolchain.toml;
  };

  packages = [
    pkgs.git
    pkgs.just
    pkgs.pkg-config
    pkgs.cmake
    pkgs.clang
    pkgs.openssl
    pkgs.duckdb
    pkgs.sqlite
    pkgs.cargo-insta
  ];

  enterShell = ''
    echo "Terminal Agent Orchestrator dev shell"
    echo "Run: cargo check"
  '';
}
