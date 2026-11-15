{
  pkgs,
  lib,
  ...
}: {
  # devenv owns the *environment* (toolchain, packages, env vars).
  # All day-to-day commands live in the justfile; run `just` to list them.

  languages.rust = {
    enable = true;
    # Keep the toolchain aligned with `rust-version` in Cargo.toml (1.90).
    channel = "stable";
    components = [
      "rustc"
      "cargo"
      "clippy"
      "rustfmt"
      "rust-analyzer"
    ];
    mold.enable = true;
  };

  packages = with pkgs; [
    # task running / watching
    just
    watchexec
    bacon

    # rust tooling
    cargo-nextest
    cargo-update
    sqlx-cli

    # native deps
    openssl
    pkg-config

    # used by justfile recipes
    ouch # vendor-compress
    wget # vendor-fetch
    gum # db-seed
    fish # db-seed
    sqlite # db-seed

    oxfmt
    ## TODO: Vite+ external installation required (`just tool-install-vp`)
  ];

  env = {
    LD_LIBRARY_PATH = lib.makeLibraryPath [pkgs.zlib];

    # `build.rs` runs `just frontend-build` unless this is 0/false/no, and
    # skips it with a warning when `just` is not installed.
    WEB_ARCHIVER_BUILD_FRONTEND = "1";

    RUST_LOG = "web_archiver=debug,tower_http=info,sqlx=warn,hyper=warn,hyper_util=warn,h2=warn,reqwest=warn,rustls=warn,lettre=warn";
    OUT_DIR = "../static/vite";

    APP_SERVER__HOST = "0.0.0.0";
    APP_SERVER__PORT = 8889;
    APP_PUBLIC_URL = "http://localhost:8889";
    APP_DATABASE__URL = "sqlite://data/db/app.sqlite3";
    APP_DATA_DIR = "./data";

    DATABASE_URL = "sqlite://data/db/app.sqlite3";
  };

  # git-hooks.hooks = {
  #   rustfmt.enable = true;
  #   clippy.enable = true;
  # };

  enterShell = ''
    echo "Run 'just' to list tasks. Crates ready to update: 'cargo update -n'"
  '';

  # See full reference at https://devenv.sh/reference/options/
}
