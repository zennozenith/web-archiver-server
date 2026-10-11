{
  pkgs,
  lib,
  config,
  # inputs,
  ...
}: {
  languages.rust = {
    enable = true;
    channel = "nightly";
    # channel = "stable";
    components = ["rustc" "cargo" "clippy" "rust-analyzer"];

    mold.enable = true;
  };

  services = {
    postgres = {
      enable = true;
      initialDatabases = [{ name = "colonist"; }];
    };
  };

  packages = with pkgs; [
    hello
    bacon
    cargo-seek
    cargo-nextest
    cargo-generate
    cargo-update

    openssl
    pkg-config

    bun
    tailwindcss_4
    oxlint
    oxfmt
    web-ext
  ];

  tasks."app:test".exec = "cargo test";

  scripts.watcher = {
    exec = ''
      watchexec -c -e rs \
      "cargo clippy && cargo test && cargo run"
    '';
    packages = [pkgs.watchexec];
  };

  
  env.LD_LIBRARY_PATH = lib.makeLibraryPath [
    pkgs.zlib
  ];

  env = {
    # DATABASE_URL = "postgres://user:pass@localhost/colonist";
    # Secrets are automatically populated from secretspec.toml
    DATABASE_URL = config.secretspec.secrets.DATABASE_URL;
  };

  # git-hooks.hooks = {
  #   rustfmt.enable = true;
  #   clippy.enable = true;
  # };

  enterShell = ''
    echo "Crates ready to update with 'cargo update -n'":
  '';
  # See full reference at https://devenv.sh/reference/options/
}
