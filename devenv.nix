{
  pkgs,
  lib,
  ...
}: {
  languages.rust = {
    enable = true;
    channel = "nightly";
    components = [
      "rustc"
      "cargo"
      "clippy"
      "rust-analyzer"
    ];

    mold.enable = true;
  };

  packages = with pkgs; [
    bacon
    cargo-nextest
    cargo-update

    openssl
    pkg-config

    watchexec
    ouch
    oxfmt
    ## TODO: Vite+ external installation required
  ];

  tasks."vendored:compress" = {
    exec = ''
      for file in *; do
          # Skip if it's a directory
          [ -d "$file" ] && continue

          case "$file" in
              *.br)
                  # Skip files ending with .br
                  ;;
              *)
                  echo "Processing: $file"
                  ouch compress $file "$file.br"
                  ;;
          esac
      done
    '';
    cwd = "./frontend/public/vendored";
  };

  tasks."app:test".exec = "cargo test";

  tasks."init:server" = {
    exec = "mkdir -p data/db";
  };

  tasks."init:frontend" = {
    exec = "vp install";
    cwd = "./frontend";
  };

  tasks."frontend:build" = {
    exec = "vpr lint && vpr fmt && vpr build";
    execIfModified = [
      "frontend"
    ];
    cwd = "./frontend";
  };

  # tasks."frontend:tailwind" = {
  #   exec = ''
  #     echo $PWD
  #     bunx @tailwindcss/cli -i ./frontend/input.css -o ./static/css/build/tailwind.css
  #   '';
  #   execIfModified = [
  #     "frontend/input.css"
  #     "templates"
  #   ];
  #   before = ["frontend:build"];
  # };

  scripts."run" = {
    exec = "devenv tasks run frontend:build && cargo run";
  };

  scripts.watcher = {
    packages = [pkgs.watchexec];
    exec = ''
      watchexec -q -c -w src/ -w frontend/ -r --stop-signal SIGKILL "cargo run"
    '';
  };

  scripts.seed-db = {
    packages = [pkgs.gum pkgs.fish pkgs.sqlite]; # See [Packages](/packages/) for an explanation.
    description = "Seeds database";
    package = pkgs.fish;
    binary = "fish";

    exec = ./seed-db.fish;
  };

  scripts.vendor-download = {
    exec = ''
      rm -rf frontend/public/vendored
      mkdir -p frontend/public/vendored
      cd frontend/public/vendored

      wget -O htmx-4.0.0.js https://cdn.jsdelivr.net/npm/htmx.org@4.0.0/dist/htmx.js
      wget -O htmx-4.0.0.min.js https://cdn.jsdelivr.net/npm/htmx.org@4.0.0/dist/htmx.min.js
      wget -O htmax-4.0.0.js https://cdn.jsdelivr.net/npm/htmx.org@4.0.0/dist/htmax.js
      wget -O htmax-4.0.0.min.js https://cdn.jsdelivr.net/npm/htmx.org@4.0.0/dist/htmax.min.js

      devenv tasks run vendored:compress
    '';
    packages = [pkgs.wget];
  };

  scripts.watch-tailwind = {
    exec = ''
      watchexec -e html,css -w frontend/input.css -w templates/ \
      -- devenv tasks run frontend:tailwind
    '';
    packages = [
      pkgs.tailwindcss_4
      pkgs.watchexec
    ];
  };

  env.LD_LIBRARY_PATH = lib.makeLibraryPath [
    pkgs.zlib
  ];

  env = {
    RUST_LOG = "info,myopic=debug";
    OUT_DIR = "../static/vite";
    APP_SERVER__HOST = "0.0.0.0";
    APP_SERVER__PORT = 8889;
    APP_DATABASE__URL = "sqlite://data/db/app.sqlite3";
    APP_DATA_DIR = "./data";
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
